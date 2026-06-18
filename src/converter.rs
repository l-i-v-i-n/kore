//! The recursive heart of the converter: turns an arbitrary
//! [`serde_json::Value`] into `.kore` text by deciding, at every level,
//! whether it looks like a table, an object, a list, or a plain scalar.

use indexmap::IndexSet;
use serde_json::{Map, Value};

use crate::infer_types::{build_types_block, infer_table_types};
use crate::serialize::{ctx_value, serialize_value};
use crate::types::{CtxValue, KoreOptions, KoreResult, Structure};

// ── classification helpers ──────────────────────────────────────────────

/// True if `value` is a non-empty array whose every element is a JSON
/// object (and not itself an array — JSON arrays report as objects in JS's
/// `typeof`, but we want the stricter "plain object" check here).
fn is_object_array(value: &Value) -> bool {
    match value {
        Value::Array(items) => {
            !items.is_empty() && items.iter().all(|item| item.is_object())
        }
        _ => false,
    }
}

/// True for any leaf JSON value: null, bool, number, or string. Arrays and
/// objects are never primitive, matching the original `isPrimitive`.
fn is_primitive(value: &Value) -> bool {
    !value.is_array() && !value.is_object()
}

/// Pad a string with trailing spaces up to `width`. Strings already at or
/// past `width` are left untouched (this only ever pads, never truncates).
fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        let mut out = String::with_capacity(width);
        out.push_str(s);
        out.push_str(&" ".repeat(width - len));
        out
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ── ctx line ─────────────────────────────────────────────────────────────

/// Render a `ctx(k=v, k2=v2)` line (with trailing newline) from an ordered
/// map of metadata fields.
fn build_ctx(fields: &indexmap::IndexMap<String, CtxValue>) -> String {
    let parts: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}={}", k, ctx_value(v)))
        .collect();
    format!("ctx({})\n", parts.join(", "))
}

// ── table converter ─────────────────────────────────────────────────────

/// Compute the column list for a table: the union of every key across all
/// rows, in first-seen order. Used both for the table body and (matching
/// behavior) for the `@types` block, so the two never disagree about which
/// columns exist.
pub(crate) fn table_columns(rows: &[Map<String, Value>]) -> Vec<String> {
    let mut seen = IndexSet::new();
    for row in rows {
        for key in row.keys() {
            seen.insert(key.clone());
        }
    }
    seen.into_iter().collect()
}

struct TableRender {
    header: String,
    body: String,
    columns: Vec<String>,
}

fn convert_table(rows: &[Map<String, Value>], block_name: &str, opts: &KoreOptions) -> TableRender {
    let columns = table_columns(rows);

    let col_widths: Vec<usize> = columns
        .iter()
        .map(|col| {
            let max_val_len = rows
                .iter()
                .map(|r| {
                    let v = r.get(col).cloned().unwrap_or(Value::Null);
                    serialize_value(&v, true).chars().count()
                })
                .max()
                .unwrap_or(0);
            col.chars().count().max(max_val_len).min(opts.max_col_width)
        })
        .collect();

    let col_list = columns.join(", ");
    let header = format!("{}[{}]:\n", block_name, col_list);

    let body_lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let cells: Vec<String> = columns
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let v = row.get(col).cloned().unwrap_or(Value::Null);
                    pad(&serialize_value(&v, true), col_widths[i])
                })
                .collect();
            format!("{}{}", opts.indent, cells.join(" | ").trim_end())
        })
        .collect();

    TableRender {
        header,
        body: body_lines.join("\n"),
        columns,
    }
}

// ── object converter ────────────────────────────────────────────────────

/// True if `value` is a primitive, or an array whose every element is a
/// primitive. Such values render inline as a single field (`key: v1, v2`)
/// rather than needing their own nested block.
fn is_scalar_field(value: &Value) -> bool {
    is_primitive(value) || matches!(value, Value::Array(items) if items.iter().all(is_primitive))
}

fn convert_object(obj: &Map<String, Value>, block_name: &str, opts: &KoreOptions) -> String {
    let mut scalars: Vec<(&String, &Value)> = Vec::new();
    let mut nested: Vec<(&String, &Value)> = Vec::new();

    for (key, val) in obj.iter() {
        if is_scalar_field(val) {
            scalars.push((key, val));
        } else {
            nested.push((key, val));
        }
    }

    let mut lines: Vec<String> = Vec::new();

    if !scalars.is_empty() && nested.is_empty() {
        // Every field is a scalar (or a scalar list) → compact ctx-style line.
        let parts: Vec<String> = scalars
            .iter()
            .map(|(k, v)| format!("{}={}", k, serialize_value(v, false)))
            .collect();
        lines.push(format!("{}({})", block_name, parts.join(", ")));
    } else {
        for (key, val) in &scalars {
            lines.push(format!("{}: {}", key, serialize_value(val, false)));
        }
        for (key, val) in &nested {
            lines.push(String::new());
            let inner = convert_any(val, key, opts);
            lines.push(inner.kore.trim().to_string());
        }
    }

    lines.join("\n")
}

// ── primitive list converter ────────────────────────────────────────────

fn convert_primitive_list(items: &[Value], block_name: &str) -> String {
    let parts: Vec<String> = items.iter().map(|v| serialize_value(v, false)).collect();
    format!("{}: {}", block_name, parts.join(", "))
}

// ── envelope detection ──────────────────────────────────────────────────

/// API responses are very often wrapped in an envelope like
/// `{ status: "ok", page: 1, items: [...] }`. When we see a plain object
/// with more than one key, and *exactly one* of those keys holds an array
/// or nested object, we treat that key as "the data" and render the rest as
/// a `ctx(...)` metadata line above it — which produces much more readable
/// output than treating the whole thing as a generic object block.
///
/// Note: this function just finds the first candidate key. The caller
/// ([`convert_any`]) is responsible for checking that it's the *only*
/// non-scalar key before using it — see the comment at that call site for
/// why that check matters.
fn find_data_key(obj: &Map<String, Value>) -> Option<String> {
    obj.iter()
        .find(|(_, v)| v.is_array() || v.is_object())
        .map(|(k, _)| k.clone())
}

// ── main recursive dispatcher ───────────────────────────────────────────

/// Convert any JSON value into `.kore` text under the given block name,
/// recursing as needed for nested arrays/objects.
pub(crate) fn convert_any(data: &Value, block_name: &str, opts: &KoreOptions) -> KoreResult {
    // array of objects → table
    if is_object_array(data) {
        let rows: Vec<Map<String, Value>> = match data {
            Value::Array(items) => items
                .iter()
                .map(|v| v.as_object().cloned().unwrap_or_default())
                .collect(),
            _ => unreachable!(),
        };

        let mut types_block = String::new();
        if opts.infer_types {
            let type_map = infer_table_types(&rows);
            let type_name = capitalize(block_name);
            let columns = table_columns(&rows);
            types_block = build_types_block(&type_name, &columns, &type_map);
            types_block.push('\n');
        }

        let table = convert_table(&rows, block_name, opts);
        return KoreResult {
            kore: format!("{}{}{}", types_block, table.header, table.body),
            structure: Structure::Table,
            columns: Some(table.columns),
            row_count: Some(rows.len()),
        };
    }

    // primitive array → list
    if let Value::Array(items) = data {
        if items.iter().all(is_primitive) {
            return KoreResult {
                kore: convert_primitive_list(items, block_name),
                structure: Structure::List,
                columns: None,
                row_count: Some(items.len()),
            };
        }

        // mixed array (some objects, some primitives, or nested arrays) →
        // best-effort table: wrap bare primitives as `{ value: <item> }` so
        // the whole thing can still render as a uniform table.
        let normalized: Vec<Value> = items
            .iter()
            .map(|item| {
                if item.is_object() {
                    item.clone()
                } else {
                    let mut m = Map::new();
                    m.insert("value".to_string(), item.clone());
                    Value::Object(m)
                }
            })
            .collect();
        return convert_any(&Value::Array(normalized), block_name, opts);
    }

    // plain object
    if let Value::Object(obj) = data {
        let keys: Vec<&String> = obj.keys().collect();
        let non_scalar_count = obj.values().filter(|v| v.is_array() || v.is_object()).count();

        // Only take the single-data-key "envelope" shortcut when there's
        // exactly one array/object-valued field. If there were two or
        // more, the original kore-js implementation still picked just the
        // *first* one and silently discarded every other nested field —
        // a real data-loss bug, not an intentional format choice. Falling
        // back to the general object renderer here means every field,
        // scalar or nested, always ends up in the output. See the README
        // changelog.
        if non_scalar_count == 1 && keys.len() > 1 {
            if let Some(data_key) = find_data_key(obj) {
                let mut meta_fields: indexmap::IndexMap<String, CtxValue> =
                    indexmap::IndexMap::new();
                for k in &keys {
                    if **k == data_key {
                        continue;
                    }
                    match obj.get(*k) {
                        Some(Value::String(s)) => {
                            meta_fields.insert((*k).clone(), CtxValue::Str(s.clone()));
                        }
                        Some(Value::Number(n)) => {
                            if let Some(f) = n.as_f64() {
                                meta_fields.insert((*k).clone(), CtxValue::Num(f));
                            }
                        }
                        Some(Value::Bool(b)) => {
                            meta_fields.insert((*k).clone(), CtxValue::Bool(*b));
                        }
                        _ => {}
                    }
                }

                let mut parts: Vec<String> = Vec::new();
                if !meta_fields.is_empty() {
                    parts.push(build_ctx(&meta_fields).trim().to_string());
                    parts.push(String::new());
                }

                let inner_value = obj.get(&data_key).cloned().unwrap_or(Value::Null);
                let inner = convert_any(&inner_value, &data_key, opts);
                parts.push(inner.kore.clone());

                return KoreResult {
                    kore: parts.join("\n"),
                    structure: inner.structure,
                    columns: inner.columns,
                    row_count: inner.row_count,
                };
            }
        }

        return KoreResult {
            kore: convert_object(obj, block_name, opts),
            structure: Structure::Object,
            columns: None,
            row_count: None,
        };
    }

    // scalar
    KoreResult {
        kore: format!("{}: {}", block_name, serialize_value(data, false)),
        structure: Structure::Scalar,
        columns: None,
        row_count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn opts(block_name: &str) -> KoreOptions {
        KoreOptions::new(block_name)
    }

    #[test]
    fn scalar_string() {
        let r = convert_any(&json!("hello"), "msg", &opts("msg"));
        assert_eq!(r.kore, "msg: hello");
        assert_eq!(r.structure, Structure::Scalar);
    }

    #[test]
    fn flat_object_is_ctx_style() {
        let r = convert_any(
            &json!({"name": "Boulder", "km": 7.5, "active": true}),
            "hike",
            &opts("hike"),
        );
        assert!(r.kore.starts_with("hike("));
        assert!(r.kore.contains("name=Boulder"));
        assert!(r.kore.contains("km=7.5"));
        assert_eq!(r.structure, Structure::Object);
    }

    #[test]
    fn primitive_list() {
        let r = convert_any(&json!(["ana", "luis", "sam"]), "friends", &opts("friends"));
        assert_eq!(r.kore, "friends: ana, luis, sam");
        assert_eq!(r.structure, Structure::List);
        assert_eq!(r.row_count, Some(3));
    }

    #[test]
    fn table_basic() {
        let data = json!([
            {"id": 1, "name": "Blue Lake Trail", "km": 7.5, "sunny": true},
            {"id": 2, "name": "Ridge Overlook", "km": 9.2, "sunny": false},
        ]);
        let r = convert_any(&data, "hikes", &opts("hikes"));
        assert_eq!(r.structure, Structure::Table);
        assert_eq!(r.row_count, Some(2));
        assert!(r.kore.contains("hikes[id, name, km, sunny]:"));
        assert!(r.kore.contains('|'));
    }

    #[test]
    fn table_null_becomes_underscore() {
        let data = json!([
            {"id": 1, "name": "Blue Lake", "rating": null},
            {"id": 2, "name": "Ridge", "rating": 4},
        ]);
        let r = convert_any(&data, "hikes", &opts("hikes"));
        assert!(r.kore.contains('_'));
    }

    #[test]
    fn envelope_extracts_meta_and_data() {
        let data = json!({
            "status": "ok",
            "page": 1,
            "hikes": [
                {"id": 1, "name": "Blue Lake", "km": 7.5},
                {"id": 2, "name": "Ridge", "km": 9.2},
            ]
        });
        let r = convert_any(&data, "hikes", &opts("hikes"));
        assert!(r.kore.contains("ctx("));
        assert!(r.kore.contains("status=ok"));
        assert!(r.kore.contains("hikes["));
        assert_eq!(r.structure, Structure::Table);
    }
}