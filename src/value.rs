/// A parsed config value.
///
/// Mappings keep insertion order and use a `Vec` instead of a hash map so
/// that parsing the same input twice always produces an equal `Value`,
/// which matters for tests and for diffing config across environments.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Sequence(Vec<Value>),
    Mapping(Vec<(String, Value)>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_sequence(&self) -> Option<&[Value]> {
        match self {
            Value::Sequence(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn as_mapping(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Mapping(entries) => Some(entries.as_slice()),
            _ => None,
        }
    }

    /// Looks up a single key in a mapping. Returns `None` for any other
    /// value kind, including when the key is absent.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_mapping()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Walks a dotted path (`"server.port"`) through nested mappings.
    pub fn path(&self, dotted: &str) -> Option<&Value> {
        let mut current = self;
        for segment in dotted.split('.') {
            current = current.get(segment)?;
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        Value::Mapping(vec![(
            "server".to_string(),
            Value::Mapping(vec![
                ("host".to_string(), Value::String("0.0.0.0".to_string())),
                ("port".to_string(), Value::Int(8080)),
            ]),
        )])
    }

    #[test]
    fn get_returns_none_for_missing_key() {
        assert_eq!(sample().get("missing"), None);
    }

    #[test]
    fn path_walks_nested_mappings() {
        assert_eq!(sample().path("server.port"), Some(&Value::Int(8080)));
    }

    #[test]
    fn path_stops_at_non_mapping() {
        assert_eq!(sample().path("server.port.deeper"), None);
    }

    #[test]
    fn as_float_widens_int() {
        assert_eq!(Value::Int(3).as_float(), Some(3.0));
    }
}
