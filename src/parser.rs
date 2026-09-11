use crate::value::Value;
use std::collections::HashMap;
use std::fmt;

/// A parse failure, with the source line it happened on (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

struct Line<'a> {
    indent: usize,
    content: &'a str,
    number: usize,
}

/// Parses a restricted, deterministic subset of YAML: block mappings,
/// block sequences, flow collections, quoted and unquoted scalars,
/// `|` and `>` block scalars, anchors and aliases, and `#` comments.
///
/// Not supported yet: multi-document streams, and the explicit
/// indentation indicator on a block scalar (`|2`) - the indentation is
/// always inferred from the first content line instead. The top-level
/// document must be a block mapping or a block sequence, since that
/// covers every real config file this library has been used for.
pub fn parse(input: &str) -> Result<Value, ParseError> {
    let raw: Vec<&str> = input.lines().collect();
    let lines = preprocess(input);
    if lines.is_empty() {
        return Ok(Value::Null);
    }
    let mut anchors: HashMap<String, Value> = HashMap::new();
    let top_indent = lines[0].indent;
    let (value, next) = parse_block(&lines, 0, top_indent, &raw, &mut anchors)?;
    if next != lines.len() {
        return Err(ParseError {
            line: lines[next].number,
            message: format!("unexpected indentation before '{}'", lines[next].content),
        });
    }
    Ok(value)
}

fn preprocess(input: &str) -> Vec<Line<'_>> {
    let mut lines = Vec::new();
    for (i, raw) in input.lines().enumerate() {
        let indent = raw.len() - raw.trim_start_matches(' ').len();
        let after_indent = &raw[indent..];
        let content = strip_comment(after_indent).trim_end();
        if content.is_empty() {
            continue;
        }
        lines.push(Line { indent, content, number: i + 1 });
    }
    lines
}

/// Truncates `s` at a `#` that starts a comment: one not inside quotes and
/// preceded by whitespace or the start of the line.
fn strip_comment(s: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => {
                let preceded_by_space = i == 0 || s[..i].ends_with(' ');
                if preceded_by_space {
                    return &s[..i];
                }
            }
            _ => {}
        }
    }
    s
}

fn is_sequence_item(content: &str) -> bool {
    content == "-" || content.starts_with("- ")
}

fn parse_block(
    lines: &[Line],
    pos: usize,
    indent: usize,
    raw: &[&str],
    anchors: &mut HashMap<String, Value>,
) -> Result<(Value, usize), ParseError> {
    if pos >= lines.len() || lines[pos].indent != indent {
        let line = lines.get(pos).map(|l| l.number).unwrap_or_else(|| lines.last().map(|l| l.number).unwrap_or(1));
        return Err(ParseError { line, message: "expected content at this indentation".to_string() });
    }
    if is_sequence_item(lines[pos].content) {
        parse_sequence(lines, pos, indent, raw, anchors)
    } else {
        parse_mapping(lines, pos, indent, raw, anchors)
    }
}

fn parse_sequence(
    lines: &[Line],
    mut pos: usize,
    indent: usize,
    raw: &[&str],
    anchors: &mut HashMap<String, Value>,
) -> Result<(Value, usize), ParseError> {
    let mut items = Vec::new();
    while pos < lines.len() && lines[pos].indent == indent && is_sequence_item(lines[pos].content) {
        let content = lines[pos].content;
        if content == "-" {
            let next_pos = pos + 1;
            if next_pos < lines.len() && lines[next_pos].indent > indent {
                let child_indent = lines[next_pos].indent;
                let (value, np) = parse_block(lines, next_pos, child_indent, raw, anchors)?;
                items.push(value);
                pos = np;
            } else {
                items.push(Value::Null);
                pos = next_pos;
            }
        } else {
            let after_dash = &content[1..];
            let leading_spaces = after_dash.len() - after_dash.trim_start().len();
            let rest = &after_dash[leading_spaces..];
            let rest_indent = indent + 1 + leading_spaces;
            let (anchor, value_part) = extract_anchor(rest, lines[pos].number)?;
            if let Some(alias_name) = value_part.strip_prefix('*') {
                if anchor.is_some() {
                    return Err(ParseError {
                        line: lines[pos].number,
                        message: "a sequence item cannot be both an anchor and an alias".to_string(),
                    });
                }
                items.push(resolve_alias(anchors, alias_name.trim(), lines[pos].number)?);
                pos += 1;
            } else if find_key_separator(value_part).is_some() {
                if anchor.is_some() {
                    return Err(ParseError {
                        line: lines[pos].number,
                        message: "anchors on inline mapping sequence items are not supported".to_string(),
                    });
                }
                let (value, np) = parse_inline_mapping_item(lines, pos, rest, rest_indent, raw, anchors)?;
                items.push(value);
                pos = np;
            } else if let Some((style, chomp)) = parse_block_indicator(value_part) {
                let (text, last_line) = read_block_scalar(raw, lines[pos].number, indent, style, chomp);
                let value = Value::String(text);
                define_anchor(anchors, anchor, &value);
                items.push(value);
                pos += 1;
                while pos < lines.len() && lines[pos].number <= last_line {
                    pos += 1;
                }
            } else if value_part.is_empty() {
                let next_pos = pos + 1;
                let value = if next_pos < lines.len() && lines[next_pos].indent > indent {
                    let child_indent = lines[next_pos].indent;
                    let (value, np) = parse_block(lines, next_pos, child_indent, raw, anchors)?;
                    pos = np;
                    value
                } else {
                    pos = next_pos;
                    Value::Null
                };
                define_anchor(anchors, anchor, &value);
                items.push(value);
            } else {
                let value = parse_scalar(value_part, lines[pos].number)?;
                define_anchor(anchors, anchor, &value);
                items.push(value);
                pos += 1;
            }
        }
    }
    Ok((Value::Sequence(items), pos))
}

/// Parses a `- key: value` sequence item, whose mapping may continue onto
/// following lines indented to line up with `key` (the column right after
/// `- `), the same way real YAML aligns an inline mapping under a dash.
fn parse_inline_mapping_item(
    lines: &[Line],
    pos: usize,
    first_line_rest: &str,
    rest_indent: usize,
    raw: &[&str],
    anchors: &mut HashMap<String, Value>,
) -> Result<(Value, usize), ParseError> {
    let mut end = pos + 1;
    while end < lines.len() && lines[end].indent >= rest_indent {
        end += 1;
    }
    let mut synthetic: Vec<Line> = Vec::with_capacity(end - pos);
    synthetic.push(Line { indent: rest_indent, content: first_line_rest, number: lines[pos].number });
    for l in &lines[pos + 1..end] {
        synthetic.push(Line { indent: l.indent, content: l.content, number: l.number });
    }
    let (value, consumed) = parse_mapping(&synthetic, 0, rest_indent, raw, anchors)?;
    Ok((value, pos + consumed))
}

fn parse_mapping(
    lines: &[Line],
    mut pos: usize,
    indent: usize,
    raw: &[&str],
    anchors: &mut HashMap<String, Value>,
) -> Result<(Value, usize), ParseError> {
    let mut entries: Vec<(String, Value)> = Vec::new();
    while pos < lines.len() && lines[pos].indent == indent && !is_sequence_item(lines[pos].content) {
        let content = lines[pos].content;
        let separator = find_key_separator(content).ok_or_else(|| ParseError {
            line: lines[pos].number,
            message: format!("expected 'key: value' but found '{}'", content),
        })?;
        let key = parse_scalar_key(content[..separator].trim());
        let rest = content[separator + 1..].trim();
        let (anchor, rest) = extract_anchor(rest, lines[pos].number)?;
        if let Some(alias_name) = rest.strip_prefix('*') {
            let value = resolve_alias(anchors, alias_name.trim(), lines[pos].number)?;
            entries.push((key, value));
            pos += 1;
        } else if let Some((style, chomp)) = parse_block_indicator(rest) {
            let (text, last_line) = read_block_scalar(raw, lines[pos].number, indent, style, chomp);
            let value = Value::String(text);
            define_anchor(anchors, anchor, &value);
            entries.push((key, value));
            pos += 1;
            while pos < lines.len() && lines[pos].number <= last_line {
                pos += 1;
            }
        } else if rest.is_empty() {
            let next_pos = pos + 1;
            let value = if next_pos < lines.len() && lines[next_pos].indent > indent {
                let child_indent = lines[next_pos].indent;
                let (value, np) = parse_block(lines, next_pos, child_indent, raw, anchors)?;
                pos = np;
                value
            } else {
                pos = next_pos;
                Value::Null
            };
            define_anchor(anchors, anchor, &value);
            entries.push((key, value));
        } else {
            let value = parse_scalar(rest, lines[pos].number)?;
            define_anchor(anchors, anchor, &value);
            entries.push((key, value));
            pos += 1;
        }
    }
    Ok((Value::Mapping(entries), pos))
}

/// Splits a leading `&name` anchor off of `rest`, returning the anchor
/// name (if present) and whatever follows it, trimmed. An anchor with no
/// name (a bare `&` with nothing before the next space) is a parse error.
fn extract_anchor<'a>(rest: &'a str, line: usize) -> Result<(Option<String>, &'a str), ParseError> {
    match rest.strip_prefix('&') {
        Some(after) => {
            let end = after.find(char::is_whitespace).unwrap_or(after.len());
            if end == 0 {
                return Err(ParseError { line, message: "expected an anchor name after '&'".to_string() });
            }
            Ok((Some(after[..end].to_string()), after[end..].trim_start()))
        }
        None => Ok((None, rest)),
    }
}

/// Records `value` under `name` for later `*name` aliases, if this value
/// was anchored at all.
fn define_anchor(anchors: &mut HashMap<String, Value>, name: Option<String>, value: &Value) {
    if let Some(name) = name {
        anchors.insert(name, value.clone());
    }
}

fn resolve_alias(anchors: &HashMap<String, Value>, name: &str, line: usize) -> Result<Value, ParseError> {
    anchors
        .get(name)
        .cloned()
        .ok_or_else(|| ParseError { line, message: format!("unknown anchor '*{}'", name) })
}

/// Finds the `:` that separates a mapping key from its value: one outside
/// quotes and flow brackets, and followed by a space or the end of the
/// line (so a URL like `http://x` inside a value, or a `k: v` pair inside
/// a flow mapping like `{k: v}`, never gets mistaken for a key separator).
fn find_key_separator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut in_single = false;
    let mut in_double = false;
    let mut depth: i32 = 0;
    for (i, c) in s.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '[' | '{' if !in_single && !in_double => depth += 1,
            ']' | '}' if !in_single && !in_double => depth -= 1,
            ':' if !in_single && !in_double && depth == 0 => {
                let at_boundary = i + 1 == s.len() || bytes[i + 1] == b' ';
                if at_boundary {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn parse_scalar_key(raw: &str) -> String {
    unquote(raw).unwrap_or_else(|| raw.to_string())
}

fn parse_scalar(raw: &str, line: usize) -> Result<Value, ParseError> {
    if raw.starts_with('[') || raw.starts_with('{') {
        return parse_flow(raw, line);
    }
    if let Some(s) = unquote(raw) {
        return Ok(Value::String(s));
    }
    Ok(scalar_from_keyword_or_number(raw))
}

fn scalar_from_keyword_or_number(raw: &str) -> Value {
    match raw {
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        _ => {}
    }
    if let Ok(i) = raw.parse::<i64>() {
        return Value::Int(i);
    }
    if let Ok(f) = raw.parse::<f64>() {
        return Value::Float(f);
    }
    Value::String(raw.to_string())
}

#[derive(Clone, Copy)]
enum BlockStyle {
    /// `|` - keep line breaks as written.
    Literal,
    /// `>` - fold single line breaks into spaces; blank lines stay as breaks.
    Folded,
}

#[derive(Clone, Copy)]
enum Chomp {
    /// `-` - drop the trailing line break entirely.
    Strip,
    /// no suffix - keep exactly one trailing line break.
    Clip,
    /// `+` - keep every trailing blank line as written.
    Keep,
}

/// Recognizes a block scalar header (`|`, `|-`, `|+`, `>`, `>-`, `>+`) in
/// an already-trimmed value. Anything else, including the explicit
/// indentation indicator (`|2`), is left for `parse_scalar` to handle as
/// a plain string.
fn parse_block_indicator(rest: &str) -> Option<(BlockStyle, Chomp)> {
    let mut chars = rest.chars();
    let style = match chars.next()? {
        '|' => BlockStyle::Literal,
        '>' => BlockStyle::Folded,
        _ => return None,
    };
    let chomp = match chars.as_str() {
        "" => Chomp::Clip,
        "-" => Chomp::Strip,
        "+" => Chomp::Keep,
        _ => return None,
    };
    Some((style, chomp))
}

/// Reads a block scalar's content lines directly from the original,
/// unprocessed source text, starting right after the `key: |` (or `- |`)
/// line at 1-based `indicator_line_number`. Reading from `raw` instead of
/// the preprocessed `Line` list matters here: comments are not stripped
/// and blank lines are not dropped inside a block scalar, unlike
/// everywhere else in this parser.
///
/// `parent_indent` is the indentation of the line carrying the `|`/`>`
/// indicator; content must be indented more than that, which is how real
/// YAML knows where the block ends. Returns the assembled string and the
/// 1-based number of the last raw line consumed by the block (equal to
/// `indicator_line_number` itself if the block turns out to be empty).
fn read_block_scalar(
    raw: &[&str],
    indicator_line_number: usize,
    parent_indent: usize,
    style: BlockStyle,
    chomp: Chomp,
) -> (String, usize) {
    let start = indicator_line_number; // raw is 0-indexed, so this is already "one past" the indicator line
    let block_indent = raw[start..]
        .iter()
        .find(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start_matches(' ').len())
        .filter(|indent| *indent > parent_indent);
    let block_indent = match block_indent {
        Some(indent) => indent,
        None => return (String::new(), start),
    };

    let mut collected: Vec<&str> = Vec::new();
    let mut i = start;
    while i < raw.len() {
        let line = raw[i];
        if line.trim().is_empty() {
            collected.push("");
            i += 1;
            continue;
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent < block_indent {
            break;
        }
        collected.push(&line[block_indent..]);
        i += 1;
    }
    let last_line = i;

    let trailing_blanks = collected.iter().rev().take_while(|l| l.is_empty()).count();
    collected.truncate(collected.len() - trailing_blanks);

    let body = match style {
        BlockStyle::Literal => collected.join("\n"),
        BlockStyle::Folded => fold_lines(&collected),
    };
    (apply_chomp(body, trailing_blanks, chomp), last_line)
}

/// Folds `>`-style content: a line break between two content lines
/// becomes a space, but a blank line still becomes a line break.
fn fold_lines(lines: &[&str]) -> String {
    let mut out = String::new();
    let mut prev_was_content = false;
    for line in lines {
        if line.is_empty() {
            out.push('\n');
            prev_was_content = false;
        } else {
            if prev_was_content {
                out.push(' ');
            }
            out.push_str(line);
            prev_was_content = true;
        }
    }
    out
}

fn apply_chomp(body: String, trailing_blanks: usize, chomp: Chomp) -> String {
    match chomp {
        Chomp::Strip => body,
        Chomp::Clip => {
            if body.is_empty() {
                body
            } else {
                body + "\n"
            }
        }
        Chomp::Keep => {
            let mut out = body;
            if !out.is_empty() {
                out.push('\n');
            }
            for _ in 0..trailing_blanks {
                out.push('\n');
            }
            out
        }
    }
}

/// Parses a `[a, b]` flow sequence or `{k: v}` flow mapping. These are
/// confined to a single logical line - there is no continuation across
/// lines - which keeps the parser a simple cursor over `raw`'s characters
/// instead of needing to look ahead into the line list.
fn parse_flow(raw: &str, line: usize) -> Result<Value, ParseError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut pos = 0;
    let value = parse_flow_value(&chars, &mut pos, line)?;
    skip_flow_ws(&chars, &mut pos);
    if pos != chars.len() {
        let rest: String = chars[pos..].iter().collect();
        return Err(ParseError { line, message: format!("unexpected trailing content '{}' after flow value", rest) });
    }
    Ok(value)
}

fn skip_flow_ws(chars: &[char], pos: &mut usize) {
    while *pos < chars.len() && chars[*pos].is_whitespace() {
        *pos += 1;
    }
}

fn parse_flow_value(chars: &[char], pos: &mut usize, line: usize) -> Result<Value, ParseError> {
    skip_flow_ws(chars, pos);
    match chars.get(*pos).copied() {
        Some('[') => parse_flow_sequence(chars, pos, line),
        Some('{') => parse_flow_mapping(chars, pos, line),
        Some('"') | Some('\'') => Ok(Value::String(parse_flow_quoted(chars, pos, line)?)),
        Some(_) => {
            let token = read_flow_token(chars, pos);
            if token.is_empty() {
                return Err(ParseError { line, message: "expected a value in flow collection".to_string() });
            }
            Ok(scalar_from_keyword_or_number(&token))
        }
        None => Err(ParseError { line, message: "expected a value but found end of input".to_string() }),
    }
}

/// Reads a bare (unquoted) flow scalar up to the next `,`, `]`, or `}`.
fn read_flow_token(chars: &[char], pos: &mut usize) -> String {
    let start = *pos;
    while *pos < chars.len() && !matches!(chars[*pos], ',' | ']' | '}') {
        *pos += 1;
    }
    chars[start..*pos].iter().collect::<String>().trim().to_string()
}

fn parse_flow_quoted(chars: &[char], pos: &mut usize, line: usize) -> Result<String, ParseError> {
    let quote = chars[*pos];
    *pos += 1;
    let mut out = String::new();
    loop {
        match chars.get(*pos).copied() {
            None => return Err(ParseError { line, message: "unterminated quoted string in flow value".to_string() }),
            Some(c) if c == quote => {
                *pos += 1;
                if quote == '\'' && chars.get(*pos).copied() == Some('\'') {
                    out.push('\'');
                    *pos += 1;
                    continue;
                }
                break;
            }
            Some('\\') if quote == '"' => {
                *pos += 1;
                match chars.get(*pos).copied() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                }
                *pos += 1;
            }
            Some(c) => {
                out.push(c);
                *pos += 1;
            }
        }
    }
    Ok(out)
}

fn parse_flow_sequence(chars: &[char], pos: &mut usize, line: usize) -> Result<Value, ParseError> {
    *pos += 1; // consume '['
    let mut items = Vec::new();
    skip_flow_ws(chars, pos);
    if chars.get(*pos).copied() == Some(']') {
        *pos += 1;
        return Ok(Value::Sequence(items));
    }
    loop {
        items.push(parse_flow_value(chars, pos, line)?);
        skip_flow_ws(chars, pos);
        match chars.get(*pos).copied() {
            Some(',') => {
                *pos += 1;
                skip_flow_ws(chars, pos);
                if chars.get(*pos).copied() == Some(']') {
                    *pos += 1;
                    break;
                }
            }
            Some(']') => {
                *pos += 1;
                break;
            }
            _ => return Err(ParseError { line, message: "expected ',' or ']' in flow sequence".to_string() }),
        }
    }
    Ok(Value::Sequence(items))
}

fn parse_flow_mapping(chars: &[char], pos: &mut usize, line: usize) -> Result<Value, ParseError> {
    *pos += 1; // consume '{'
    let mut entries: Vec<(String, Value)> = Vec::new();
    skip_flow_ws(chars, pos);
    if chars.get(*pos).copied() == Some('}') {
        *pos += 1;
        return Ok(Value::Mapping(entries));
    }
    loop {
        skip_flow_ws(chars, pos);
        let key = match chars.get(*pos).copied() {
            Some('"') | Some('\'') => parse_flow_quoted(chars, pos, line)?,
            _ => {
                let start = *pos;
                while *pos < chars.len() && chars[*pos] != ':' {
                    *pos += 1;
                }
                if *pos >= chars.len() {
                    return Err(ParseError { line, message: "expected ':' in flow mapping entry".to_string() });
                }
                chars[start..*pos].iter().collect::<String>().trim().to_string()
            }
        };
        skip_flow_ws(chars, pos);
        if chars.get(*pos).copied() != Some(':') {
            return Err(ParseError { line, message: "expected ':' after key in flow mapping".to_string() });
        }
        *pos += 1;
        let value = parse_flow_value(chars, pos, line)?;
        entries.push((key, value));
        skip_flow_ws(chars, pos);
        match chars.get(*pos).copied() {
            Some(',') => {
                *pos += 1;
                skip_flow_ws(chars, pos);
                if chars.get(*pos).copied() == Some('}') {
                    *pos += 1;
                    break;
                }
            }
            Some('}') => {
                *pos += 1;
                break;
            }
            _ => return Err(ParseError { line, message: "expected ',' or '}' in flow mapping".to_string() }),
        }
    }
    Ok(Value::Mapping(entries))
}

fn unquote(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if raw.len() >= 2 && bytes[0] == b'"' && bytes[raw.len() - 1] == b'"' {
        Some(unescape_double(&raw[1..raw.len() - 1]))
    } else if raw.len() >= 2 && bytes[0] == b'\'' && bytes[raw.len() - 1] == b'\'' {
        Some(raw[1..raw.len() - 1].replace("''", "'"))
    } else {
        None
    }
}

fn unescape_double(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flat_mapping() {
        let value = parse("host: localhost\nport: 8080\n").unwrap();
        assert_eq!(value.get("host"), Some(&Value::String("localhost".to_string())));
        assert_eq!(value.get("port"), Some(&Value::Int(8080)));
    }

    #[test]
    fn parses_nested_mapping() {
        let value = parse("server:\n  host: localhost\n  port: 8080\n").unwrap();
        let server = value.get("server").unwrap();
        assert_eq!(server.get("host"), Some(&Value::String("localhost".to_string())));
    }

    #[test]
    fn parses_sequence_of_scalars() {
        let value = parse("tags:\n  - alpha\n  - beta\n").unwrap();
        let tags = value.get("tags").unwrap().as_sequence().unwrap();
        assert_eq!(tags, &[Value::String("alpha".to_string()), Value::String("beta".to_string())]);
    }

    #[test]
    fn parses_sequence_of_mappings() {
        let input = "servers:\n  -\n    host: a\n    port: 1\n  -\n    host: b\n    port: 2\n";
        let value = parse(input).unwrap();
        let servers = value.get("servers").unwrap().as_sequence().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].get("host"), Some(&Value::String("a".to_string())));
        assert_eq!(servers[1].get("port"), Some(&Value::Int(2)));
    }

    #[test]
    fn parses_inline_mapping_sequence_item() {
        let input = "servers:\n  - host: a\n    port: 1\n  - host: b\n    port: 2\n";
        let value = parse(input).unwrap();
        let servers = value.get("servers").unwrap().as_sequence().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].get("host"), Some(&Value::String("a".to_string())));
        assert_eq!(servers[0].get("port"), Some(&Value::Int(1)));
        assert_eq!(servers[1].get("host"), Some(&Value::String("b".to_string())));
        assert_eq!(servers[1].get("port"), Some(&Value::Int(2)));
    }

    #[test]
    fn inline_mapping_sequence_item_with_single_key() {
        let value = parse("names:\n  - first: alice\n  - first: bob\n").unwrap();
        let names = value.get("names").unwrap().as_sequence().unwrap();
        assert_eq!(names[0].get("first"), Some(&Value::String("alice".to_string())));
        assert_eq!(names[1].get("first"), Some(&Value::String("bob".to_string())));
    }

    #[test]
    fn inline_mapping_sequence_item_with_nested_block() {
        let input = "items:\n  - name: a\n    meta:\n      owner: x\n  - name: b\n";
        let value = parse(input).unwrap();
        let items = value.get("items").unwrap().as_sequence().unwrap();
        assert_eq!(items[0].get("name"), Some(&Value::String("a".to_string())));
        assert_eq!(items[0].path("meta.owner"), Some(&Value::String("x".to_string())));
        assert_eq!(items[1].get("name"), Some(&Value::String("b".to_string())));
    }

    #[test]
    fn strips_full_line_and_trailing_comments() {
        let value = parse("# a comment\nport: 8080 # inline comment\n").unwrap();
        assert_eq!(value.get("port"), Some(&Value::Int(8080)));
    }

    #[test]
    fn hash_inside_quotes_is_not_a_comment() {
        let value = parse("motd: \"say # hi\"\n").unwrap();
        assert_eq!(value.get("motd"), Some(&Value::String("say # hi".to_string())));
    }

    #[test]
    fn colon_inside_value_is_not_a_key_separator() {
        let value = parse("url: http://example.com\n").unwrap();
        assert_eq!(value.get("url"), Some(&Value::String("http://example.com".to_string())));
    }

    #[test]
    fn double_quoted_strings_support_escapes() {
        let value = parse("line: \"a\\nb\"\n").unwrap();
        assert_eq!(value.get("line"), Some(&Value::String("a\nb".to_string())));
    }

    #[test]
    fn null_true_false_and_numbers_are_typed() {
        let value = parse("a: null\nb: true\nc: false\nd: 42\ne: 3.5\n").unwrap();
        assert_eq!(value.get("a"), Some(&Value::Null));
        assert_eq!(value.get("b"), Some(&Value::Bool(true)));
        assert_eq!(value.get("c"), Some(&Value::Bool(false)));
        assert_eq!(value.get("d"), Some(&Value::Int(42)));
        assert_eq!(value.get("e"), Some(&Value::Float(3.5)));
    }

    #[test]
    fn quoted_scalar_stays_a_string_even_if_numeric() {
        let value = parse("version: \"1.0\"\n").unwrap();
        assert_eq!(value.get("version"), Some(&Value::String("1.0".to_string())));
    }

    #[test]
    fn reports_line_number_on_malformed_entry() {
        let err = parse("host: localhost\nthis is not a key value pair\n").unwrap_err();
        assert_eq!(err.line, 2);
    }

    #[test]
    fn empty_document_is_null() {
        assert_eq!(parse("").unwrap(), Value::Null);
        assert_eq!(parse("\n\n# just a comment\n").unwrap(), Value::Null);
    }

    #[test]
    fn parses_flow_sequence_of_scalars() {
        let value = parse("tags: [alpha, beta, 3]\n").unwrap();
        let tags = value.get("tags").unwrap().as_sequence().unwrap();
        assert_eq!(
            tags,
            &[Value::String("alpha".to_string()), Value::String("beta".to_string()), Value::Int(3)]
        );
    }

    #[test]
    fn parses_empty_flow_collections() {
        let value = parse("a: []\nb: {}\n").unwrap();
        assert_eq!(value.get("a"), Some(&Value::Sequence(vec![])));
        assert_eq!(value.get("b"), Some(&Value::Mapping(vec![])));
    }

    #[test]
    fn parses_flow_mapping() {
        let value = parse("point: {x: 1, y: 2}\n").unwrap();
        let point = value.get("point").unwrap();
        assert_eq!(point.get("x"), Some(&Value::Int(1)));
        assert_eq!(point.get("y"), Some(&Value::Int(2)));
    }

    #[test]
    fn parses_nested_flow_collections() {
        let value = parse("data: [{k: v}, [1, 2]]\n").unwrap();
        let data = value.get("data").unwrap().as_sequence().unwrap();
        assert_eq!(data[0].get("k"), Some(&Value::String("v".to_string())));
        assert_eq!(data[1].as_sequence(), Some(&[Value::Int(1), Value::Int(2)][..]));
    }

    #[test]
    fn flow_collection_respects_quoted_commas_and_colons() {
        let value = parse("tags: [\"a, b\", 'c: d']\n").unwrap();
        let tags = value.get("tags").unwrap().as_sequence().unwrap();
        assert_eq!(tags, &[Value::String("a, b".to_string()), Value::String("c: d".to_string())]);
    }

    #[test]
    fn flow_sequence_as_bare_sequence_item() {
        let value = parse("rows:\n  - [1, 2]\n  - [3, 4]\n").unwrap();
        let rows = value.get("rows").unwrap().as_sequence().unwrap();
        assert_eq!(rows[0].as_sequence(), Some(&[Value::Int(1), Value::Int(2)][..]));
        assert_eq!(rows[1].as_sequence(), Some(&[Value::Int(3), Value::Int(4)][..]));
    }

    #[test]
    fn flow_mapping_as_bare_sequence_item() {
        let value = parse("rows:\n  - {name: a, val: 1}\n  - {name: b, val: 2}\n").unwrap();
        let rows = value.get("rows").unwrap().as_sequence().unwrap();
        assert_eq!(rows[0].get("name"), Some(&Value::String("a".to_string())));
        assert_eq!(rows[1].get("val"), Some(&Value::Int(2)));
    }

    #[test]
    fn unterminated_flow_sequence_is_an_error() {
        let err = parse("tags: [a, b\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn trailing_content_after_flow_value_is_an_error() {
        let err = parse("tags: [a, b] extra\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn literal_block_scalar_keeps_line_breaks() {
        let value = parse("script: |\n  line one\n  line two\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String("line one\nline two\n".to_string())));
    }

    #[test]
    fn folded_block_scalar_joins_lines_with_spaces() {
        let value = parse("summary: >\n  line one\n  line two\n").unwrap();
        assert_eq!(value.get("summary"), Some(&Value::String("line one line two\n".to_string())));
    }

    #[test]
    fn folded_block_scalar_keeps_blank_line_as_break() {
        let value = parse("summary: >\n  first para\n\n  second para\n").unwrap();
        assert_eq!(value.get("summary"), Some(&Value::String("first para\nsecond para\n".to_string())));
    }

    #[test]
    fn block_scalar_strip_chomping_drops_trailing_newline() {
        let value = parse("script: |-\n  line one\n  line two\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String("line one\nline two".to_string())));
    }

    #[test]
    fn block_scalar_keep_chomping_preserves_trailing_blank_lines() {
        let value = parse("script: |+\nother: 1\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String(String::new())));
        assert_eq!(value.get("other"), Some(&Value::Int(1)));

        let value = parse("script: |+\n  content\n\n\nother: 1\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String("content\n\n\n".to_string())));
        assert_eq!(value.get("other"), Some(&Value::Int(1)));
    }

    #[test]
    fn block_scalar_ends_at_lower_indentation() {
        let value = parse("script: |\n  line one\n  line two\nother: 1\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String("line one\nline two\n".to_string())));
        assert_eq!(value.get("other"), Some(&Value::Int(1)));
    }

    #[test]
    fn empty_block_scalar_is_empty_string() {
        let value = parse("script: |\nother: 1\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String(String::new())));
        assert_eq!(value.get("other"), Some(&Value::Int(1)));
    }

    #[test]
    fn block_scalar_preserves_hash_and_indentation_inside_content() {
        let value = parse("script: |\n  #!/bin/sh\n  echo hi # not a comment\n").unwrap();
        assert_eq!(value.get("script"), Some(&Value::String("#!/bin/sh\necho hi # not a comment\n".to_string())));
    }

    #[test]
    fn block_scalar_as_sequence_item() {
        let input = "notes:\n  - |\n    line one\n    line two\n  - second\n";
        let value = parse(input).unwrap();
        let notes = value.get("notes").unwrap().as_sequence().unwrap();
        assert_eq!(notes[0], Value::String("line one\nline two\n".to_string()));
        assert_eq!(notes[1], Value::String("second".to_string()));
    }

    #[test]
    fn block_scalar_nested_under_mapping_key() {
        let input = "server:\n  motd: |\n    hello\n    world\n  port: 8080\n";
        let value = parse(input).unwrap();
        assert_eq!(value.path("server.motd"), Some(&Value::String("hello\nworld\n".to_string())));
        assert_eq!(value.path("server.port"), Some(&Value::Int(8080)));
    }

    #[test]
    fn alias_reuses_anchored_scalar() {
        let value = parse("a: &x 5\nb: *x\n").unwrap();
        assert_eq!(value.get("a"), Some(&Value::Int(5)));
        assert_eq!(value.get("b"), Some(&Value::Int(5)));
    }

    #[test]
    fn alias_reuses_anchored_mapping() {
        let input = "defaults: &defaults\n  host: localhost\n  port: 8080\nserver: *defaults\n";
        let value = parse(input).unwrap();
        assert_eq!(value.path("server.host"), Some(&Value::String("localhost".to_string())));
        assert_eq!(value.path("server.port"), Some(&Value::Int(8080)));
    }

    #[test]
    fn alias_reuses_anchored_flow_collection() {
        let value = parse("a: &pair [1, 2]\nb: *pair\n").unwrap();
        assert_eq!(value.get("a"), value.get("b"));
        assert_eq!(value.get("b").unwrap().as_sequence(), Some(&[Value::Int(1), Value::Int(2)][..]));
    }

    #[test]
    fn alias_used_as_sequence_item() {
        let input = "base: &base\n  role: worker\nworkers:\n  - *base\n  - *base\n";
        let value = parse(input).unwrap();
        let workers = value.get("workers").unwrap().as_sequence().unwrap();
        assert_eq!(workers[0].get("role"), Some(&Value::String("worker".to_string())));
        assert_eq!(workers[1].get("role"), Some(&Value::String("worker".to_string())));
    }

    #[test]
    fn anchor_and_alias_on_sequence_scalar_items() {
        let value = parse("tags:\n  - &t alpha\n  - *t\n").unwrap();
        let tags = value.get("tags").unwrap().as_sequence().unwrap();
        assert_eq!(tags, &[Value::String("alpha".to_string()), Value::String("alpha".to_string())]);
    }

    #[test]
    fn later_anchor_overwrites_earlier_one_with_the_same_name() {
        let value = parse("a: &x 1\nb: &x 2\nc: *x\n").unwrap();
        assert_eq!(value.get("c"), Some(&Value::Int(2)));
    }

    #[test]
    fn unknown_alias_is_an_error() {
        let err = parse("a: *missing\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn alias_cannot_reference_an_anchor_defined_later() {
        let err = parse("a: *x\nb: &x 1\n").unwrap_err();
        assert_eq!(err.line, 1);
    }

    #[test]
    fn anchor_without_a_name_is_an_error() {
        let err = parse("a: & 1\n").unwrap_err();
        assert_eq!(err.line, 1);
    }
}
