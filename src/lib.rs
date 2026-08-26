//! Parsing and merging for a deterministic subset of YAML, meant for
//! layered application config (defaults + environment overrides).
//!
//! Every public function here is pure: given the same input it always
//! returns the same output, and none of them touch the filesystem or any
//! other outside state. Reading files is left to the caller so the parsing
//! and merging logic stays trivial to unit test.

mod merge;
mod parser;
mod value;

pub use merge::merge;
pub use parser::{parse, ParseError};
pub use value::Value;
