use std::fmt;

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

/// Renders a `Value` back to YAML text that this crate's own parser can
/// read again - `parse(&value.to_string())` round-trips whenever `value`
/// is a `Mapping` or `Sequence` at the top level, which is the only shape
/// `parse` accepts there.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        match self {
            Value::Mapping(entries) if entries.is_empty() => out.push_str("{}\n"),
            Value::Mapping(entries) => write_mapping(&mut out, entries, 0),
            Value::Sequence(items) if items.is_empty() => out.push_str("[]\n"),
            Value::Sequence(items) => write_sequence(&mut out, items, 0),
            scalar => {
                out.push_str(&format_scalar(scalar));
                out.push('\n');
            }
        }
        f.write_str(&out)
    }
}

fn write_mapping(out: &mut String, entries: &[(String, Value)], indent: usize) {
    for (key, value) in entries {
        out.push_str(&" ".repeat(indent));
        out.push_str(&format_key(key));
        out.push(':');
        write_child(out, value, indent);
    }
}

fn write_sequence(out: &mut String, items: &[Value], indent: usize) {
    for item in items {
        out.push_str(&" ".repeat(indent));
        out.push('-');
        write_child(out, item, indent);
    }
}

/// Writes what comes after a mapping's `key:` or a sequence's `-`: either
/// an indented nested block on following lines, or the rest of the same
/// line for a scalar or empty collection.
fn write_child(out: &mut String, value: &Value, indent: usize) {
    match value {
        Value::Mapping(entries) if !entries.is_empty() => {
            out.push('\n');
            write_mapping(out, entries, indent + 2);
        }
        Value::Mapping(_) => out.push_str(" {}\n"),
        Value::Sequence(items) if !items.is_empty() => {
            out.push('\n');
            write_sequence(out, items, indent + 2);
        }
        Value::Sequence(_) => out.push_str(" []\n"),
        scalar => {
            out.push(' ');
            out.push_str(&format_scalar(scalar));
            out.push('\n');
        }
    }
}

fn format_scalar(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(true) => "true".to_string(),
        Value::Bool(false) => "false".to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => format_float(*f),
        Value::String(s) => format_string_scalar(s),
        Value::Sequence(_) | Value::Mapping(_) => unreachable!("collections are written by write_child"),
    }
}

/// `f64`'s default `Display` drops the fractional part for whole numbers
/// (`3.0` becomes `"3"`), which would parse back as `Value::Int`. Force a
/// decimal point so the value keeps its `Float` type on re-parse.
fn format_float(f: f64) -> String {
    if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{:.1}", f)
    } else {
        f.to_string()
    }
}

fn format_string_scalar(s: &str) -> String {
    if value_needs_quoting(s) {
        quote(s)
    } else {
        s.to_string()
    }
}

fn format_key(s: &str) -> String {
    if structurally_needs_quoting(s) {
        quote(s)
    } else {
        s.to_string()
    }
}

/// True when a bare `s` would parse back as something other than the
/// string it is, because it matches a keyword or looks like a number.
fn value_needs_quoting(s: &str) -> bool {
    if matches!(s, "~" | "null" | "Null" | "NULL" | "true" | "True" | "TRUE" | "false" | "False" | "FALSE") {
        return true;
    }
    if s.parse::<i64>().is_ok() || s.parse::<f64>().is_ok() {
        return true;
    }
    structurally_needs_quoting(s)
}

/// True when a bare `s` would confuse the line-oriented parser regardless
/// of what value it is meant to represent: as a key it would break the
/// `key: value` split, as a value it would be mistaken for a sequence
/// item, a comment, or a flow collection.
fn structurally_needs_quoting(s: &str) -> bool {
    if s.is_empty() || s.trim() != s {
        return true;
    }
    if s.contains('\n') || s.contains('\t') || s.contains('#') {
        return true;
    }
    if s.contains(": ") || s.ends_with(':') {
        return true;
    }
    if s == "-" || s.starts_with("- ") {
        return true;
    }
    matches!(s.chars().next(), Some('[' | ']' | '{' | '}' | ',' | '\'' | '"' | '&' | '*' | '!' | '|' | '>' | '%' | '@' | '`'))
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
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

    #[test]
    fn display_renders_nested_mapping() {
        assert_eq!(sample().to_string(), "server:\n  host: 0.0.0.0\n  port: 8080\n");
    }

    #[test]
    fn display_renders_sequence_under_a_key() {
        let value = Value::Mapping(vec![(
            "tags".to_string(),
            Value::Sequence(vec![Value::String("a".to_string()), Value::String("b".to_string())]),
        )]);
        assert_eq!(value.to_string(), "tags:\n  - a\n  - b\n");
    }

    #[test]
    fn display_renders_empty_collections_inline() {
        let value = Value::Mapping(vec![
            ("a".to_string(), Value::Sequence(vec![])),
            ("b".to_string(), Value::Mapping(vec![])),
        ]);
        assert_eq!(value.to_string(), "a: []\nb: {}\n");
    }

    #[test]
    fn display_quotes_strings_that_would_change_type() {
        let value = Value::Mapping(vec![
            ("a".to_string(), Value::String("42".to_string())),
            ("b".to_string(), Value::String("true".to_string())),
            ("c".to_string(), Value::String("null".to_string())),
            ("d".to_string(), Value::String("".to_string())),
        ]);
        assert_eq!(value.to_string(), "a: \"42\"\nb: \"true\"\nc: \"null\"\nd: \"\"\n");
    }

    #[test]
    fn display_quotes_strings_with_structural_characters() {
        let value = Value::Mapping(vec![
            ("a".to_string(), Value::String("say # hi".to_string())),
            ("b".to_string(), Value::String("a: b".to_string())),
            ("c".to_string(), Value::String("- item".to_string())),
        ]);
        assert_eq!(value.to_string(), "a: \"say # hi\"\nb: \"a: b\"\nc: \"- item\"\n");
    }

    #[test]
    fn display_escapes_backslashes_quotes_and_control_characters() {
        let value = Value::String("a\\b\"c\nd\te".to_string());
        assert_eq!(value.to_string(), "\"a\\\\b\\\"c\\nd\\te\"\n");
    }

    #[test]
    fn display_keeps_whole_floats_as_floats() {
        let value = Value::Mapping(vec![("ratio".to_string(), Value::Float(3.0))]);
        assert_eq!(value.to_string(), "ratio: 3.0\n");
    }

    #[test]
    fn display_round_trips_through_parse() {
        let value = Value::Mapping(vec![
            ("name".to_string(), Value::String("42".to_string())),
            (
                "server".to_string(),
                Value::Mapping(vec![
                    ("host".to_string(), Value::String("0.0.0.0".to_string())),
                    ("port".to_string(), Value::Int(8080)),
                    ("ratio".to_string(), Value::Float(1.5)),
                    ("enabled".to_string(), Value::Bool(true)),
                    ("note".to_string(), Value::Null),
                ]),
            ),
            (
                "tags".to_string(),
                Value::Sequence(vec![
                    Value::String("say # hi".to_string()),
                    Value::String("a: b".to_string()),
                    Value::Sequence(vec![Value::Int(1), Value::Int(2)]),
                ]),
            ),
        ]);
        let rendered = value.to_string();
        assert_eq!(crate::parser::parse(&rendered).unwrap(), value);
    }
}
