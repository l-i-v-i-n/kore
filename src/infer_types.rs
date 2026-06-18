//! Type inference for the optional `@types { ... }` block that can be
//! emitted above a table when [`crate::KoreOptions::infer_types`] is set.
//!
//! This walks every row of a table and, for each column, figures out the
//! narrowest `.kore` scalar type that fits every value seen (promoting to a
//! wider numeric type on conflict, falling back to `str` if the column mixes
//! incompatible kinds, and appending `?` if any row was missing the field
//! or had it set to `null`).

use indexmap::{IndexMap, IndexSet};
use serde_json::Value;

/// Infer the `.kore` type tag for a single value.
///
/// - `null` reports as `str?` on its own (a lone null tells us nothing about
///   the column's real type beyond "nullable"; [`infer_table_types`] is
///   responsible for combining this with whatever other rows reveal).
/// - Strings are checked against a couple of common shapes (`YYYY-MM-DD`
///   dates, `http(s)://` URLs) before falling back to plain `str`.
/// - Numbers are bucketed into the smallest unsigned/signed integer type
///   that fits, or `f32`/`f64` for non-integers and non-finite values.
/// - Arrays report as `[]str` (element-type inference is intentionally kept
///   simple here; mixed-type arrays are common enough in real API data that
///   guessing further would just be noise).
pub fn infer_value_type(value: &Value) -> String {
    match value {
        Value::Null => "str?".to_string(),
        Value::Bool(_) => "bool".to_string(),
        Value::String(s) => {
            if is_date_like(s) {
                "date".to_string()
            } else if is_url_like(s) {
                "url".to_string()
            } else {
                "str".to_string()
            }
        }
        Value::Number(n) => infer_number_type(n),
        Value::Array(_) => "[]str".to_string(),
        Value::Object(_) => "str".to_string(),
    }
}

fn is_date_like(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[0..4].iter().all(|c| c.is_ascii_digit())
        && b[4] == b'-'
        && b[5..7].iter().all(|c| c.is_ascii_digit())
        && b[7] == b'-'
        && b[8..10].iter().all(|c| c.is_ascii_digit())
}

fn is_url_like(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn infer_number_type(n: &serde_json::Number) -> String {
    if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return "f64".to_string();
        }
    }
    if let Some(i) = n.as_i64() {
        return integer_type(i);
    }
    if let Some(u) = n.as_u64() {
        return unsigned_type(u);
    }
    // Has a fractional part (or doesn't fit in i64/u64) → floating point.
    "f32".to_string()
}

fn unsigned_type(u: u64) -> String {
    if u <= u8::MAX as u64 {
        "u8".to_string()
    } else if u <= u16::MAX as u64 {
        "u16".to_string()
    } else if u <= u32::MAX as u64 {
        "u32".to_string()
    } else {
        "u64".to_string()
    }
}

fn integer_type(i: i64) -> String {
    if i >= 0 {
        return unsigned_type(i as u64);
    }
    if i >= i32::MIN as i64 {
        "i32".to_string()
    } else {
        "i64".to_string()
    }
}

/// The widening order used to resolve a column that contains more than one
/// numeric type across rows (e.g. some rows have `u8` ids, others `u16`).
const NUMERIC_ORDER: [&str; 8] = ["u8", "u16", "u32", "u64", "i32", "i64", "f32", "f64"];

/// Pick the widest type among a set of detected (non-nullable-suffixed)
/// types for one column. If every type is numeric, promote to the widest
/// one seen. If the column mixes numeric with something else (e.g. some
/// rows are numbers, others are strings), fall back to `str` rather than
/// picking arbitrarily.
fn resolve_types(types: &IndexSet<String>) -> String {
    if types.is_empty() {
        return "str?".to_string();
    }
    if types.len() == 1 {
        return types.iter().next().unwrap().clone();
    }
    let all_numeric = types.iter().all(|t| NUMERIC_ORDER.contains(&t.as_str()));
    if all_numeric {
        let widest = types
            .iter()
            .map(|t| NUMERIC_ORDER.iter().position(|n| n == t).unwrap())
            .max()
            .unwrap();
        return NUMERIC_ORDER[widest].to_string();
    }
    "str".to_string()
}

/// Infer a `.kore` type per column across every row of a table.
///
/// Unlike the original kore-js implementation — which only looked at the
/// *first* row's keys and so silently dropped any column that the first row
/// happened to be missing — this scans the full set of keys across **all**
/// rows, the same union [`crate::converter::table_columns`] uses for the
/// table body. That keeps the `@types` block and the table header in sync
/// even when rows have inconsistent shapes (a common case with real API
/// data). See the README changelog for details.
pub fn infer_table_types(rows: &[serde_json::Map<String, Value>]) -> IndexMap<String, String> {
    let mut columns: IndexSet<String> = IndexSet::new();
    for row in rows {
        for key in row.keys() {
            columns.insert(key.clone());
        }
    }

    let mut result = IndexMap::new();
    for col in &columns {
        let mut seen_types: IndexSet<String> = IndexSet::new();
        let mut has_null = false;

        for row in rows {
            match row.get(col) {
                None => has_null = true,
                Some(Value::Null) => has_null = true,
                Some(v) => {
                    seen_types.insert(infer_value_type(v));
                }
            }
        }

        let mut resolved = resolve_types(&seen_types);
        if has_null && !resolved.ends_with('?') {
            resolved.push('?');
        }
        result.insert(col.clone(), resolved);
    }

    result
}

/// Render the `@types { TypeName { field: type, ... } }` block for a table.
///
/// `type_name` is typically the capitalized block name (e.g. `Hikes` for a
/// block named `hikes`), and `columns` should be passed in the same order
/// used for the table header so the two blocks read consistently top to
/// bottom.
pub fn build_types_block(
    type_name: &str,
    columns: &[String],
    type_map: &IndexMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str("@types {\n");
    out.push_str("  ");
    out.push_str(type_name);
    out.push_str(" {\n");
    for col in columns {
        let ty = type_map
            .get(col)
            .map(|s| s.as_str())
            .unwrap_or("str");
        out.push_str("    ");
        out.push_str(col);
        out.push_str(": ");
        out.push_str(ty);
        out.push('\n');
    }
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn infers_basic_types() {
        assert_eq!(infer_value_type(&json!(true)), "bool");
        assert_eq!(infer_value_type(&json!(1)), "u8");
        assert_eq!(infer_value_type(&json!(300)), "u16");
        assert_eq!(infer_value_type(&json!(7.5)), "f32");
        assert_eq!(infer_value_type(&json!("hello")), "str");
        assert_eq!(infer_value_type(&json!("2024-01-01")), "date");
        assert_eq!(infer_value_type(&json!("https://example.com")), "url");
        assert_eq!(infer_value_type(&Value::Null), "str?");
    }

    #[test]
    fn nullable_field_gets_question_mark() {
        let rows = vec![
            obj(json!({"name": "ana", "score": null})),
            obj(json!({"name": "luis", "score": 5})),
        ];
        let types = infer_table_types(&rows);
        assert_eq!(types.get("score").unwrap(), "u8?");
    }

    #[test]
    fn union_of_columns_across_rows() {
        // First row lacks `extra`; second row has it. The fixed
        // implementation must still report a type for `extra`.
        let rows = vec![
            obj(json!({"id": 1})),
            obj(json!({"id": 2, "extra": "x"})),
        ];
        let types = infer_table_types(&rows);
        assert_eq!(types.get("extra").unwrap(), "str?");
    }

    #[test]
    fn numeric_widening() {
        let rows = vec![obj(json!({"id": 1})), obj(json!({"id": 999999}))];
        let types = infer_table_types(&rows);
        assert_eq!(types.get("id").unwrap(), "u32");
    }
}