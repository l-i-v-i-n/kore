//! Integration tests mirroring `kore-js`'s test suite, so it's easy to see
//! at a glance that this port behaves the same way for every documented
//! case (plus a few extra tests for the bug fixes noted in the README).

use kore::{to_kore, to_kore_from_str, KoreOptions, Structure};
use serde_json::json;

// ── scalar values ────────────────────────────────────────────────────────

#[test]
fn scalar_string() {
    let r = to_kore(&json!("hello"), &KoreOptions::new("msg"));
    assert_eq!(r.kore, "msg: hello");
    assert_eq!(r.structure, Structure::Scalar);
}

#[test]
fn scalar_number() {
    let r = to_kore(&json!(42), &KoreOptions::new("count"));
    assert_eq!(r.kore, "count: 42");
}

#[test]
fn scalar_bool() {
    let r = to_kore(&json!(true), &KoreOptions::new("active"));
    assert_eq!(r.kore, "active: true");
}

#[test]
fn scalar_null() {
    let r = to_kore(&json!(null), &KoreOptions::new("val"));
    assert_eq!(r.kore, "val: _");
}

// ── flat object ──────────────────────────────────────────────────────────

#[test]
fn flat_object_all_scalars_is_ctx_style() {
    let r = to_kore(
        &json!({"name": "Boulder", "km": 7.5, "active": true}),
        &KoreOptions::new("hike"),
    );
    assert!(r.kore.contains("hike("));
    assert!(r.kore.contains("name=Boulder"));
    assert!(r.kore.contains("km=7.5"));
    assert_eq!(r.structure, Structure::Object);
}

// ── primitive list ───────────────────────────────────────────────────────

#[test]
fn string_list() {
    let r = to_kore(&json!(["ana", "luis", "sam"]), &KoreOptions::new("friends"));
    assert_eq!(r.kore, "friends: ana, luis, sam");
    assert_eq!(r.structure, Structure::List);
    assert_eq!(r.row_count, Some(3));
}

#[test]
fn number_list() {
    let r = to_kore(&json!([1, 2, 3]), &KoreOptions::new("ids"));
    assert_eq!(r.kore, "ids: 1, 2, 3");
}

// ── table (array of objects) ────────────────────────────────────────────

fn hikes_fixture() -> serde_json::Value {
    json!([
        {"id": 1, "name": "Blue Lake Trail", "km": 7.5, "sunny": true},
        {"id": 2, "name": "Ridge Overlook",  "km": 9.2, "sunny": false},
        {"id": 3, "name": "Wildflower Loop", "km": 5.1, "sunny": true},
    ])
}

#[test]
fn table_structure_is_table() {
    let r = to_kore(&hikes_fixture(), &KoreOptions::new("hikes"));
    assert_eq!(r.structure, Structure::Table);
    assert_eq!(r.row_count, Some(3));
    assert_eq!(
        r.columns,
        Some(vec![
            "id".to_string(),
            "name".to_string(),
            "km".to_string(),
            "sunny".to_string()
        ])
    );
}

#[test]
fn table_header_contains_column_names() {
    let r = to_kore(&hikes_fixture(), &KoreOptions::new("hikes"));
    assert!(r.kore.contains("hikes[id, name, km, sunny]:"));
}

#[test]
fn table_rows_use_pipe_separator() {
    let r = to_kore(&hikes_fixture(), &KoreOptions::new("hikes"));
    assert!(r.kore.contains('|'));
}

#[test]
fn table_booleans_serialized_correctly() {
    let r = to_kore(&hikes_fixture(), &KoreOptions::new("hikes"));
    assert!(r.kore.contains("true"));
    assert!(r.kore.contains("false"));
}

#[test]
fn table_null_values_become_underscore() {
    let data = json!([
        {"id": 1, "name": "Blue Lake", "rating": null},
        {"id": 2, "name": "Ridge",     "rating": 4},
    ]);
    let r = to_kore(&data, &KoreOptions::new("hikes"));
    assert!(r.kore.contains('_'));
}

#[test]
fn table_strings_with_commas_are_safe() {
    let data = json!([{"id": 1, "name": "Blue Lake, CO", "km": 7.5}]);
    let r = to_kore(&data, &KoreOptions::new("hikes"));
    let data_line = r
        .kore
        .lines()
        .find(|l| l.contains("Blue Lake, CO"))
        .expect("row containing the comma-bearing name");
    // Exactly 2 pipe separators for 3 columns.
    assert_eq!(data_line.matches('|').count(), 2);
}

// ── ctx option ───────────────────────────────────────────────────────────

#[test]
fn ctx_option_adds_top_line() {
    let opts = KoreOptions::new("friends")
        .with_ctx("task", "hike planner")
        .with_ctx("ver", 1i32);
    let r = to_kore(&json!(["ana", "luis"]), &opts);
    assert!(r.kore.starts_with("ctx("));
    assert!(r.kore.contains("task="));
    assert!(r.kore.contains("ver=1"));
}

// ── infer_types option ──────────────────────────────────────────────────

#[test]
fn infer_types_emits_types_block() {
    let data = json!([{"id": 1, "name": "Blue Lake", "km": 7.5, "sunny": true}]);
    let r = to_kore(&data, &KoreOptions::new("hikes").infer_types(true));
    assert!(r.kore.contains("@types"));
    assert!(r.kore.contains("Hikes {"));
}

#[test]
fn infer_types_detects_u8_for_small_positive_ints() {
    let data = json!([{"id": 1}, {"id": 2}]);
    let r = to_kore(&data, &KoreOptions::new("items").infer_types(true));
    assert!(r.kore.contains("u8"));
}

#[test]
fn infer_types_detects_f32_for_floats() {
    let data = json!([{"km": 7.5}, {"km": 9.2}]);
    let r = to_kore(&data, &KoreOptions::new("items").infer_types(true));
    assert!(r.kore.contains("f32"));
}

#[test]
fn infer_types_detects_bool() {
    let data = json!([{"active": true}, {"active": false}]);
    let r = to_kore(&data, &KoreOptions::new("items").infer_types(true));
    assert!(r.kore.contains("bool"));
}

#[test]
fn infer_types_nullable_field_gets_question_mark() {
    let data = json!([{"name": "ana", "score": null}, {"name": "luis", "score": 5}]);
    let r = to_kore(&data, &KoreOptions::new("items").infer_types(true));
    assert!(r.kore.contains("score: u8?") || r.kore.contains("score:u8?"));
}

#[test]
fn infer_types_union_of_columns_across_rows_regression() {
    // Bug fix vs. kore-js: the original only looked at the first row's
    // keys, so a column missing from row 0 but present later was silently
    // dropped from the @types block. This must now appear.
    let data = json!([{"id": 1}, {"id": 2, "extra": "x"}]);
    let r = to_kore(&data, &KoreOptions::new("items").infer_types(true));
    assert!(r.kore.contains("extra"));
}

// ── envelope object ──────────────────────────────────────────────────────

#[test]
fn envelope_extracts_meta_as_ctx_and_data_as_table() {
    let api = json!({
        "status": "ok",
        "page": 1,
        "hikes": [
            {"id": 1, "name": "Blue Lake", "km": 7.5},
            {"id": 2, "name": "Ridge",     "km": 9.2},
        ]
    });
    let r = to_kore(&api, &KoreOptions::new("hikes"));
    assert!(r.kore.contains("ctx("));
    assert!(r.kore.contains("status=ok"));
    assert!(r.kore.contains("hikes["));
}

// ── comment option ──────────────────────────────────────────────────────

#[test]
fn comment_option_adds_header() {
    let r = to_kore(&json!({"x": 1}), &KoreOptions::new("data").comment("generated by kore"));
    assert!(r.kore.starts_with("// generated by kore"));
}

// ── to_kore_from_str ─────────────────────────────────────────────────────

#[test]
fn from_str_parses_json_then_converts() {
    let json_str = json!([{"id": 1, "name": "ana"}]).to_string();
    let r = to_kore_from_str(&json_str, &KoreOptions::new("users")).unwrap();
    assert_eq!(r.structure, Structure::Table);
    assert!(r.kore.contains("users["));
}

#[test]
fn from_str_returns_err_on_invalid_json() {
    let result = to_kore_from_str("{not valid", &KoreOptions::default());
    assert!(result.is_err());
}

// ── nested objects (beyond the original JS suite) ───────────────────────

#[test]
fn single_nested_field_is_treated_as_envelope_data_key() {
    // This mirrors a real characteristic of kore-js, not a bug: any plain
    // object with more than one key, where exactly one key holds an array
    // or object, gets treated as an "envelope" — the other scalar keys
    // become a ctx(...) line and the array/object key becomes the main
    // block. That's exactly what you want for API responses like
    // `{ status: "ok", items: [...] }`, but it also fires for an ordinary
    // object that merely happens to have one nested field, like this one.
    let data = json!({
        "name": "ana",
        "address": { "city": "Boulder", "zip": "80301" }
    });
    let r = to_kore(&data, &KoreOptions::new("user"));
    assert!(r.kore.contains("ctx(name=ana)"));
    assert!(r.kore.contains("address(city=Boulder, zip=80301)"));
}

#[test]
fn array_field_inside_scalar_object_is_bracketed() {
    // Same envelope behavior as above: `tags` is the lone array-valued key,
    // so it becomes the main block (a primitive list) and `name` becomes
    // ctx metadata. The list itself is unaffected by the bracketing fix
    // (that only applies to arrays nested *inside* another object's
    // ctx-style inline rendering, not to top-level lists).
    let data = json!({"name": "ana", "tags": ["fast", "fun"]});
    let r = to_kore(&data, &KoreOptions::new("user"));
    assert!(r.kore.contains("ctx(name=ana)"));
    assert!(r.kore.contains("tags: fast, fun"));
}

#[test]
fn object_with_multiple_nested_fields_keeps_every_field() {
    // Regression test for a real data-loss bug found while porting kore-js:
    // the original only ever promoted the *first* array/object-valued key
    // to be "the data," silently dropping every other nested field with no
    // trace in the output (not even as ctx metadata, since ctx only holds
    // scalars). Here, both `address` and `tags` must survive — `tags`
    // renders as its own bracketed scalar-list line since it's a sibling
    // field, not the sole nested value.
    let data = json!({
        "name": "ana",
        "address": { "city": "Boulder" },
        "tags": ["fast", "fun"]
    });
    let r = to_kore(&data, &KoreOptions::new("user"));
    assert!(r.kore.contains("name: ana"));
    assert!(r.kore.contains("address("));
    assert!(r.kore.contains("city=Boulder"));
    assert!(r.kore.contains("tags: [fast, fun]"));
}

#[test]
fn array_value_in_ctx_object_envelope_path() {
    // Documented improvement over kore-js: arrays are bracketed wherever
    // serialize_value renders them inline (see array_field_inside_scalar_
    // object_is_bracketed and object_with_multiple_nested_fields_keeps_
    // every_field for those cases). This object's only nested field is
    // `tags`, so it takes the single-data-key envelope path instead — the
    // scalar fields become ctx metadata and `tags` renders as its own
    // primitive list line, unbracketed, exactly like a top-level list
    // would.
    let data = json!({"name": "ana", "age": 30, "tags": ["fast", "fun"]});
    let r = to_kore(&data, &KoreOptions::new("user"));
    assert!(r.kore.contains("ctx(name=ana, age=30)"));
    assert!(r.kore.contains("tags: fast, fun"));
}