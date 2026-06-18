//! Core types shared across the crate: conversion options, results,
//! and the small helper enums used to describe detected structure.

use indexmap::IndexMap;

/// The shape of data that was detected while converting.
///
/// Mirrors the `structure` field from the original kore-js `KoreResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Structure {
    /// An array of objects, rendered as a `name[col, col2]:` table block.
    Table,
    /// A plain object, rendered as either `name(k=v)` or a multi-line block.
    Object,
    /// An array of primitive values, rendered as `name: a, b, c`.
    List,
    /// A single primitive value, rendered as `name: value`.
    Scalar,
}

impl Structure {
    /// Lowercase string form, matching the JS string union
    /// (`"table" | "object" | "list" | "scalar"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            Structure::Table => "table",
            Structure::Object => "object",
            Structure::List => "list",
            Structure::Scalar => "scalar",
        }
    }
}

impl std::fmt::Display for Structure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A scalar value usable inside a `ctx(...)` block: string, number, or bool.
///
/// This exists separately from [`serde_json::Value`] because `ctx` only
/// ever holds flat metadata — never arrays, objects, or null.
#[derive(Debug, Clone, PartialEq)]
pub enum CtxValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

impl From<&str> for CtxValue {
    fn from(s: &str) -> Self {
        CtxValue::Str(s.to_string())
    }
}
impl From<String> for CtxValue {
    fn from(s: String) -> Self {
        CtxValue::Str(s)
    }
}
impl From<f64> for CtxValue {
    fn from(n: f64) -> Self {
        CtxValue::Num(n)
    }
}
impl From<i64> for CtxValue {
    fn from(n: i64) -> Self {
        CtxValue::Num(n as f64)
    }
}
impl From<i32> for CtxValue {
    fn from(n: i32) -> Self {
        CtxValue::Num(n as f64)
    }
}
impl From<bool> for CtxValue {
    fn from(b: bool) -> Self {
        CtxValue::Bool(b)
    }
}

/// Options controlling how JSON is converted into `.kore` text.
///
/// All fields have sensible defaults via [`KoreOptions::default`], so most
/// callers only need to set `block_name` (and maybe `infer_types`):
///
/// ```
/// use kore::KoreOptions;
///
/// let opts = KoreOptions::new("hikes").infer_types(true);
/// ```
#[derive(Debug, Clone)]
pub struct KoreOptions {
    /// Block name used for the root value (e.g. `"hikes"` in `hikes[...]:`).
    /// Default: `"data"`.
    pub block_name: String,
    /// Optional metadata rendered as a `ctx(k=v, ...)` line at the top of
    /// the document.
    pub ctx: IndexMap<String, CtxValue>,
    /// Whether table rows use a `|` separator between columns.
    /// Default: `true`. (Reserved for future plain-text mode; currently
    /// `.kore` tables always use pipes, matching kore-js.)
    pub pipe_rows: bool,
    /// Whether to emit an `@types { ... }` block above tables, inferring a
    /// type per column from the row data. Default: `false`.
    pub infer_types: bool,
    /// Indentation string used for nested content and table rows.
    /// Default: two spaces.
    pub indent: String,
    /// Optional `// comment` header line written above everything else.
    pub comment: Option<String>,
    /// Maximum column width used when aligning table cells.
    /// Default: `30`.
    pub max_col_width: usize,
}

impl Default for KoreOptions {
    fn default() -> Self {
        KoreOptions {
            block_name: "data".to_string(),
            ctx: IndexMap::new(),
            pipe_rows: true,
            infer_types: false,
            indent: "  ".to_string(),
            comment: None,
            max_col_width: 30,
        }
    }
}

impl KoreOptions {
    /// Start from defaults with a given root block name.
    pub fn new(block_name: impl Into<String>) -> Self {
        KoreOptions {
            block_name: block_name.into(),
            ..Default::default()
        }
    }

    /// Builder-style setter for `infer_types`.
    pub fn infer_types(mut self, value: bool) -> Self {
        self.infer_types = value;
        self
    }

    /// Builder-style setter for `comment`.
    pub fn comment(mut self, value: impl Into<String>) -> Self {
        self.comment = Some(value.into());
        self
    }

    /// Builder-style setter for `indent`.
    pub fn indent(mut self, value: impl Into<String>) -> Self {
        self.indent = value.into();
        self
    }

    /// Builder-style setter for `max_col_width`.
    pub fn max_col_width(mut self, value: usize) -> Self {
        self.max_col_width = value;
        self
    }

    /// Builder-style setter to add a single `ctx` entry.
    pub fn with_ctx(mut self, key: impl Into<String>, value: impl Into<CtxValue>) -> Self {
        self.ctx.insert(key.into(), value.into());
        self
    }
}

/// The output of converting JSON to `.kore`.
///
/// Mirrors kore-js's `KoreResult`: the rendered text plus a little metadata
/// describing what was detected, so callers can branch on it without
/// re-parsing the output string.
#[derive(Debug, Clone)]
pub struct KoreResult {
    /// The full rendered `.kore` text.
    pub kore: String,
    /// The detected top-level structure.
    pub structure: Structure,
    /// Column names, present only when `structure == Structure::Table`.
    pub columns: Option<Vec<String>>,
    /// Row count, present when `structure` is `Table` or `List`.
    pub row_count: Option<usize>,
}