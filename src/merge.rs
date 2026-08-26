use crate::value::Value;

/// Deep-merges `patch` on top of `base`.
///
/// Mappings are merged key by key, recursing into nested mappings so a
/// small override file only has to name the keys it actually changes.
/// Anything else (scalars, sequences, or a kind mismatch between the two
/// sides) is resolved by taking `patch` as-is, since there is no sane
/// element-wise merge for a list or a scalar.
pub fn merge(base: &Value, patch: &Value) -> Value {
    match (base, patch) {
        (Value::Mapping(base_entries), Value::Mapping(patch_entries)) => {
            let mut merged: Vec<(String, Value)> = base_entries.clone();
            for (key, patch_value) in patch_entries {
                match merged.iter_mut().find(|(k, _)| k == key) {
                    Some(existing) => existing.1 = merge(&existing.1, patch_value),
                    None => merged.push((key.clone(), patch_value.clone())),
                }
            }
            Value::Mapping(merged)
        }
        _ => patch.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping(entries: Vec<(&str, Value)>) -> Value {
        Value::Mapping(entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    #[test]
    fn merge_overwrites_matching_scalar_keys() {
        let base = mapping(vec![("port", Value::Int(8080))]);
        let patch = mapping(vec![("port", Value::Int(9090))]);
        assert_eq!(merge(&base, &patch), mapping(vec![("port", Value::Int(9090))]));
    }

    #[test]
    fn merge_keeps_base_keys_not_present_in_patch() {
        let base = mapping(vec![("host", Value::String("localhost".to_string())), ("port", Value::Int(8080))]);
        let patch = mapping(vec![("port", Value::Int(9090))]);
        let result = merge(&base, &patch);
        assert_eq!(result.get("host"), Some(&Value::String("localhost".to_string())));
        assert_eq!(result.get("port"), Some(&Value::Int(9090)));
    }

    #[test]
    fn merge_recurses_into_nested_mappings() {
        let base = mapping(vec![("server", mapping(vec![("host", Value::String("localhost".to_string())), ("port", Value::Int(8080))]))]);
        let patch = mapping(vec![("server", mapping(vec![("port", Value::Int(9090))]))]);
        let result = merge(&base, &patch);
        let server = result.get("server").unwrap();
        assert_eq!(server.get("host"), Some(&Value::String("localhost".to_string())));
        assert_eq!(server.get("port"), Some(&Value::Int(9090)));
    }

    #[test]
    fn merge_replaces_sequences_wholesale() {
        let base = mapping(vec![("tags", Value::Sequence(vec![Value::String("a".to_string())]))]);
        let patch = mapping(vec![("tags", Value::Sequence(vec![Value::String("b".to_string())]))]);
        let result = merge(&base, &patch);
        assert_eq!(result.get("tags"), Some(&Value::Sequence(vec![Value::String("b".to_string())])));
    }

    #[test]
    fn merge_with_non_mapping_patch_replaces_base_entirely() {
        let base = mapping(vec![("port", Value::Int(8080))]);
        let patch = Value::Null;
        assert_eq!(merge(&base, &patch), Value::Null);
    }
}
