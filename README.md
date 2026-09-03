# yaml-layers

Most apps end up with config split across a few files: a `defaults.yaml`
checked into the repo, a `production.yaml` that overrides a handful of
keys, maybe a debug override on top of that. Pulling those together
usually means reaching for a full YAML crate plus some hand-rolled merge
function that only gets tested by accident, in production.

`yaml-layers` is a small standard-library-only crate that does two things:
parses a deterministic subset of YAML into a `Value`, and deep-merges one
`Value` on top of another. Both are plain functions of their inputs - no
file I/O, no globals, no hidden state - so they are trivial to unit test
and safe to call from anywhere, including build scripts or `#[test]`
blocks.

Loading files is left to the caller on purpose:

```rust
use std::fs;
use yaml_layers::{merge, parse};

fn load_config(env: &str) -> yaml_layers::Value {
    let defaults = fs::read_to_string("config/defaults.yaml").unwrap();
    let overrides = fs::read_to_string(format!("config/{env}.yaml")).unwrap();

    let base = parse(&defaults).unwrap();
    let patch = parse(&overrides).unwrap();
    merge(&base, &patch)
}

fn main() {
    let config = load_config("production");
    let port = config.path("server.port").and_then(|v| v.as_int()).unwrap_or(8080);
    println!("listening on {port}");
}
```

Given:

```yaml
# config/defaults.yaml
server:
  host: 0.0.0.0
  port: 8080
tags:
  - default

# config/production.yaml
server:
  port: 443
```

`load_config("production")` returns a config where `server.host` is still
`"0.0.0.0"` (untouched by the override) and `server.port` is `443`
(replaced by it). `tags` would be replaced wholesale rather than merged
element-by-element if the override set it, since there is no single right
way to merge a list.

## Supported YAML

- block mappings and block sequences, indented with spaces
- inline `- key: value` sequence items, with the mapping continuing on
  following lines aligned under `key`
- flow collections, `[a, b, c]` and `{k: v, k2: v2}`, including nested
  ones like `[{k: v}, [1, 2]]` - each flow collection must fit on a
  single line
- scalars: strings (quoted or bare), integers, floats, booleans, null
- single- and double-quoted strings, with `\n`, `\t`, `\"`, `\\` escapes
  in double-quoted strings and `''` as an escaped quote in single-quoted
  ones
- `#` comments, both full-line and trailing (ignored inside quotes)

Not supported yet, and not silently mangled - unrecognized syntax is a
parse error, not a guess:

- anchors, aliases, and tags
- multi-document streams (`---`)

## Layout

- `src/value.rs` - the `Value` enum and read-only accessors (`get`, `path`,
  `as_str`, ...)
- `src/parser.rs` - `parse(&str) -> Result<Value, ParseError>`
- `src/merge.rs` - `merge(&Value, &Value) -> Value`

## License

MIT, see `LICENSE`.
