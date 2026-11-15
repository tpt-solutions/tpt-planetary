//! Hand-written JSON helpers for `--format json` output.
//!
//! The CLI does not depend on `serde_json` directly. Values that come from
//! the array crate's metadata are serialised with their own `Display` impl,
//! which produces compact, correctly escaped JSON.

use std::fmt::Write as _;

/// Returns `s` as a JSON string literal, including the surrounding quotes.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Returns an optional string as a JSON string literal or `null`.
pub fn opt_string(s: Option<&str>) -> String {
    s.map_or_else(|| "null".to_string(), string)
}

/// Returns a slice of unsigned integers as a JSON array.
pub fn u64_array(values: &[u64]) -> String {
    let items: Vec<String> = values.iter().map(u64::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// Returns a slice of `f64` values as a JSON array. Values must be finite.
pub fn f64_array(values: &[f64]) -> String {
    let items: Vec<String> = values.iter().map(f64::to_string).collect();
    format!("[{}]", items.join(", "))
}

/// Joins already-serialised members into a JSON object.
pub fn object(members: &[(&str, String)]) -> String {
    let body: Vec<String> = members
        .iter()
        .map(|(k, v)| format!("{}: {}", string(k), v))
        .collect();
    format!("{{{}}}", body.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_quotes_backslashes_and_controls() {
        assert_eq!(string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(string("line\nTab\t"), r#""line\nTab\t""#);
        assert_eq!(string("\u{01}"), r#""\u0001""#);
        assert_eq!(string("caf\u{e9}"), "\"caf\u{e9}\"");
    }

    #[test]
    fn builds_objects_and_arrays() {
        assert_eq!(u64_array(&[4, 6]), "[4, 6]");
        assert_eq!(u64_array(&[]), "[]");
        assert_eq!(
            object(&[("a", "1".to_string()), ("b\"", opt_string(None))]),
            r#"{"a": 1, "b\"": null}"#
        );
    }
}
