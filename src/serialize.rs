//! Rules for turning a single JSON value into its `.kore` text form.
//!
//! This is the lowest-level layer: every other module eventually calls
//! [`serialize_value`] or [`ctx_value`] to render a leaf value. Keeping the
//! quoting/escaping rules in one place means the table, object, and list
//! renderers all stay consistent.

use serde_json::Value;

/// Characters that force a string to be quoted when it appears inside a
/// table cell, because a bare `|` would otherwise be mistaken for a column
/// separator.
fn needs_pipe_quote(s: &str) -> bool {
    s.contains('|')
}

/// Strings that start with a digit, underscore, or quote character are
/// ambiguous in table cells (they could be misread as a number, a `_` null
/// marker, or an already-quoted value), so they get wrapped in quotes too.
fn needs_leading_char_quote(s: &str) -> bool {
    matches!(s.chars().next(), Some(c) if c == '_' || c == '"' || c == '\'' || c.is_ascii_digit())
}

/// Escape a string for safe placement inside double quotes: backslashes and
/// quote characters are escaped, matching common `.kore`-reader expectations.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render a JSON number the way `.kore` expects: integers with no trailing
/// `.0`, and everything else in plain decimal form (no scientific notation),
/// matching how kore-js's `String(number)` behaves for ordinary API data.
fn format_number(n: &serde_json::Number) -> String {
    if let Some(i) = n.as_i64() {
        return i.to_string();
    }
    if let Some(u) = n.as_u64() {
        return u.to_string();
    }
    if let Some(f) = n.as_f64() {
        if f.is_nan() || f.is_infinite() {
            // serde_json can't represent these anyway, but guard for safety
            // if a caller builds a Value by hand.
            return "_".to_string();
        }
        if f == f.trunc() && f.abs() < 1e15 {
            return format!("{}", f as i64);
        }
        // Use the original textual form from the parser when available —
        // serde_json's Number Display already avoids exponential notation
        // for typical magnitudes and preserves the parsed precision.
        return n.to_string();
    }
    n.to_string()
}

/// Serialize a single value to its `.kore` string representation.
///
/// `in_table` controls the extra quoting rules that only apply inside table
/// cells (pipe-safety and leading-character ambiguity); everywhere else
/// (ctx-style object fields, list items, nested inline objects) those rules
/// don't apply because there's no `|` delimiter to protect against.
///
/// Nested arrays and objects always recurse with `in_table = false` for
/// their own elements, since an array element is never itself a table cell.
pub fn serialize_value(value: &Value, in_table: bool) -> String {
    match value {
        Value::Null => "_".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => format_number(n),
        Value::String(s) => {
            if in_table && (needs_pipe_quote(s) || needs_leading_char_quote(s)) {
                quote(s)
            } else {
                s.clone()
            }
        }
        Value::Array(items) => {
            let rendered: Vec<String> = items.iter().map(|v| serialize_value(v, false)).collect();
            // Wrapping in brackets removes the ambiguity you'd get from a
            // bare comma-joined list sitting next to other comma-joined
            // fields (e.g. `tags=a, b, c` reads as three separate fields).
            // kore-js omits the brackets; we add them as a small, additive
            // clarity fix — see the README changelog.
            format!("[{}]", rendered.join(", "))
        }
        Value::Object(map) => {
            let parts: Vec<String> = map
                .iter()
                .map(|(k, v)| format!("{}={}", k, serialize_value(v, false)))
                .collect();
            parts.join(", ")
        }
    }
}

/// Characters that force quoting inside a `ctx(...)` block: whitespace,
/// comma, `=`, parens, and double quotes would otherwise be ambiguous with
/// the `key=value, key2=value2` syntax of the block itself.
fn ctx_needs_quote(s: &str) -> bool {
    s.chars()
        .any(|c| c.is_whitespace() || matches!(c, ',' | '=' | '(' | ')' | '"'))
}

/// Render a `ctx()` field value (string, number, or bool). Numbers and
/// booleans are never quoted; strings are quoted only if they contain a
/// character that would otherwise be ambiguous in `ctx(k=v, ...)` syntax.
pub fn ctx_value(value: &crate::types::CtxValue) -> String {
    use crate::types::CtxValue;
    match value {
        CtxValue::Bool(b) => b.to_string(),
        CtxValue::Num(n) => {
            if *n == n.trunc() && n.abs() < 1e15 {
                format!("{}", *n as i64)
            } else {
                n.to_string()
            }
        }
        CtxValue::Str(s) => {
            if ctx_needs_quote(s) {
                quote(s)
            } else {
                s.clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_becomes_underscore() {
        assert_eq!(serialize_value(&Value::Null, false), "_");
    }

    #[test]
    fn bools() {
        assert_eq!(serialize_value(&json!(true), false), "true");
        assert_eq!(serialize_value(&json!(false), false), "false");
    }

    #[test]
    fn integers_have_no_decimal() {
        assert_eq!(serialize_value(&json!(42), false), "42");
        assert_eq!(serialize_value(&json!(-7), false), "-7");
    }

    #[test]
    fn floats_render_plainly() {
        assert_eq!(serialize_value(&json!(7.5), false), "7.5");
    }

    #[test]
    fn plain_string_unquoted() {
        assert_eq!(serialize_value(&json!("hello"), false), "hello");
        assert_eq!(serialize_value(&json!("hello"), true), "hello");
    }

    #[test]
    fn table_string_with_pipe_is_quoted() {
        assert_eq!(serialize_value(&json!("a|b"), true), "\"a|b\"");
        // outside a table, no need to quote
        assert_eq!(serialize_value(&json!("a|b"), false), "a|b");
    }

    #[test]
    fn table_string_with_leading_digit_is_quoted() {
        assert_eq!(serialize_value(&json!("123abc"), true), "\"123abc\"");
    }

    #[test]
    fn comma_in_string_is_left_alone_but_safe_in_table() {
        // Commas are fine bare in table cells (pipe is the delimiter there);
        // this matches kore-js's behavior exactly.
        assert_eq!(serialize_value(&json!("Blue Lake, CO"), true), "Blue Lake, CO");
    }

    #[test]
    fn arrays_are_bracketed() {
        assert_eq!(serialize_value(&json!(["a", "b", "c"]), false), "[a, b, c]");
    }

    #[test]
    fn nested_object_inlines_as_kv() {
        assert_eq!(
            serialize_value(&json!({"x": 1, "y": 2}), false),
            "x=1, y=2"
        );
    }
}