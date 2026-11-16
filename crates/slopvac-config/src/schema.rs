//! Structural validation of `slopvac.toml` and the front-matter `slopvac:` block.
//!
//! The shapes mirror `schemas/v3/config.schema.json` and
//! `schemas/v3/frontmatter.schema.json`. The conformance corpus under
//! `schemas/v3/conformance/` is the shared test of both.

use std::ops::Range;

use crate::diag::{Code, Diagnostic, closest};
use crate::tree::{Entry, Node, Value};

/// The shape a value must have.
#[derive(Debug, Clone)]
pub enum Shape {
    /// A string with at least one character.
    NonEmptyStr,
    /// A string that `check` accepts. `name` completes "a valid ...".
    Pattern {
        /// Name of the accepted form, such as "pack id".
        name: &'static str,
        /// The accept test.
        check: fn(&str) -> bool,
    },
    /// A boolean.
    Bool,
    /// An integer within optional bounds.
    Int {
        /// Inclusive lower bound.
        min: Option<i64>,
        /// Inclusive upper bound.
        max: Option<i64>,
    },
    /// A density budget: a non-negative number, or `none` to clear the budget of a lower layer.
    Budget,
    /// An integer or float within optional bounds.
    Num {
        /// Inclusive lower bound.
        min: Option<f64>,
        /// Inclusive upper bound.
        max: Option<f64>,
    },
    /// One of a closed set of strings.
    Choice {
        /// The accepted strings.
        values: &'static [&'static str],
        /// Advice when a table appears where a bare value belongs.
        table_help: Option<&'static str>,
    },
    /// The schema version, which must be 3.
    Version,
    /// A list of one item shape.
    List {
        /// Shape of each item.
        item: Box<Shape>,
        /// Minimum item count.
        min_items: usize,
        /// Whether items must differ.
        unique: bool,
    },
    /// A closed table.
    Table(TableShape),
    /// A table whose keys are data, such as the rule names under `rules`.
    Map {
        /// Accept test for a key.
        key: fn(&str) -> bool,
        /// Name of the accepted key form.
        key_name: &'static str,
        /// Shape of each value.
        value: Box<Shape>,
    },
    /// The `scalar` shape for a non-table value, the `table` shape for a table.
    Either {
        /// Shape of a non-table value.
        scalar: Box<Shape>,
        /// Shape of a table.
        table: Box<Shape>,
    },
}

/// A table with named fields. Keys outside `fields` are errors, except `x-` keys.
#[derive(Debug, Clone)]
pub struct TableShape {
    /// The fields.
    pub fields: Vec<(&'static str, Shape)>,
    /// Keys that must be present.
    pub required: Vec<&'static str>,
    /// Where the table sits, when whole-run keys are out of place in it.
    pub scope: Option<&'static str>,
}

/// Keys that apply to the whole run, never to one file.
const PROJECT_WIDE: &[&str] = &[
    "guidance",
    "network",
    "review",
    "packs",
    "plugins",
    "styles",
    "extends",
    "frontmatter",
    "mode",
    "exclude",
];

fn table(fields: Vec<(&'static str, Shape)>, required: &[&'static str]) -> Shape {
    Shape::Table(TableShape {
        fields,
        required: required.to_vec(),
        scope: None,
    })
}

fn scoped(
    scope: &'static str,
    fields: Vec<(&'static str, Shape)>,
    required: &[&'static str],
) -> Shape {
    Shape::Table(TableShape {
        fields,
        required: required.to_vec(),
        scope: Some(scope),
    })
}

fn choice(values: &'static [&'static str]) -> Shape {
    Shape::Choice {
        values,
        table_help: None,
    }
}

fn list(item: Shape) -> Shape {
    Shape::List {
        item: Box::new(item),
        min_items: 0,
        unique: false,
    }
}

fn strings() -> Shape {
    list(Shape::NonEmptyStr)
}

fn is_pack_id(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_ref(s: &str) -> bool {
    match s.split_once(':') {
        Some((pack, name)) => is_pack_id(pack) && is_pack_id(name),
        None => is_pack_id(s),
    }
}

fn is_rule_key(s: &str) -> bool {
    let name = match s.split_once(':') {
        Some((pack, name)) if is_pack_id(pack) => name,
        Some(_) => return false,
        None => s,
    };
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn pack_id() -> Shape {
    Shape::Pattern {
        name: "pack id (lowercase letters, digits and hyphens)",
        check: is_pack_id,
    }
}

fn reference() -> Shape {
    Shape::Pattern {
        name: "name, bare or as `<pack>:<name>`",
        check: is_ref,
    }
}

const LEVELS: &[&str] = &["error", "warning", "suggestion", "off"];
const PROFILES: &[&str] = &["strict", "normal", "relaxed"];
const LOCALES: &[&str] = &["en-US", "en-GB", "und"];
const SETTINGS_KEYS: &[&str] = &[
    "genre",
    "profile",
    "locale",
    "thresholds",
    "categories",
    "rules",
    "terms",
];

fn level() -> Shape {
    choice(LEVELS)
}

fn rule_level() -> Shape {
    Shape::Choice {
        values: LEVELS,
        table_help: Some("a rule takes a bare level, such as `\"core:no-dash\" = \"off\"`"),
    }
}

fn profile() -> Shape {
    Shape::Either {
        scalar: Box::new(choice(PROFILES)),
        table: Box::new(table(
            vec![
                ("name", choice(PROFILES)),
                (
                    "severity_shift",
                    Shape::Int {
                        min: Some(-2),
                        max: Some(2),
                    },
                ),
            ],
            &[],
        )),
    }
}

fn non_negative() -> Shape {
    Shape::Num {
        min: Some(0.0),
        max: None,
    }
}

fn thresholds() -> Shape {
    let count = Shape::Int {
        min: Some(0),
        max: None,
    };
    table(
        vec![
            ("max_total_per_100_words", non_negative()),
            ("max_errors", count.clone()),
            ("max_warnings", count.clone()),
            (
                "min_score",
                Shape::Num {
                    min: Some(0.0),
                    max: Some(100.0),
                },
            ),
            ("max_unicode_dashes", count),
        ],
        &[],
    )
}

fn category_settings() -> Shape {
    Shape::Either {
        scalar: Box::new(level()),
        table: Box::new(table(
            vec![
                ("severity", level()),
                ("minimum_severity", level()),
                ("max_per_100_words", Shape::Budget),
                ("weight", non_negative()),
            ],
            &[],
        )),
    }
}

fn keyed(value: Shape) -> Shape {
    Shape::Map {
        key: is_rule_key,
        key_name: "rule or category name",
        value: Box::new(value),
    }
}

fn terms() -> Shape {
    table(
        vec![
            ("allow", strings()),
            ("files", strings()),
            ("blocklists", strings()),
        ],
        &[],
    )
}

fn settings_fields() -> Vec<(&'static str, Shape)> {
    vec![
        ("profile", profile()),
        ("locale", choice(LOCALES)),
        ("thresholds", thresholds()),
        ("categories", keyed(category_settings())),
        ("rules", keyed(rule_level())),
        ("terms", terms()),
    ]
}

fn files() -> Shape {
    Shape::List {
        item: Box::new(Shape::NonEmptyStr),
        min_items: 1,
        unique: false,
    }
}

fn source() -> Shape {
    table(
        vec![
            ("id", pack_id()),
            ("source", Shape::NonEmptyStr),
            ("version", Shape::NonEmptyStr),
        ],
        &["id", "source"],
    )
}

/// The shape of `slopvac.toml`.
#[must_use]
pub fn config_shape() -> Shape {
    let mut override_fields = settings_fields();
    override_fields.push(("files", files()));
    let mut genre_fields = settings_fields();
    genre_fields.push(("files", files()));
    genre_fields.push(("genre", reference()));
    table(
        vec![
            ("schema_version", Shape::Version),
            ("mode", choice(&["prose", "code-comments"])),
            ("extends", strings()),
            ("styles", list(reference())),
            ("exclude", strings()),
            ("packs", list(source())),
            ("plugins", list(source())),
            (
                "genres",
                list(scoped("a genre block", genre_fields, &["genre", "files"])),
            ),
            (
                "frontmatter",
                table(
                    vec![
                        ("enabled", Shape::Bool),
                        (
                            "allow",
                            Shape::List {
                                item: Box::new(choice(SETTINGS_KEYS)),
                                min_items: 0,
                                unique: true,
                            },
                        ),
                    ],
                    &[],
                ),
            ),
            (
                "guidance",
                table(
                    vec![
                        ("level", choice(&["brief", "standard", "full"])),
                        ("write", choice(&["auto", "off"])),
                        ("review", choice(&["auto", "ask", "off"])),
                        ("review_depth", choice(&["self", "subagent", "adversary"])),
                        ("report", choice(&["ask", "off"])),
                    ],
                    &[],
                ),
            ),
            (
                "overrides",
                list(scoped("an override", override_fields, &["files"])),
            ),
            (
                "network",
                table(
                    vec![("offline", Shape::Bool), ("update_check", Shape::Bool)],
                    &[],
                ),
            ),
            (
                "review",
                table(
                    vec![
                        ("provider", pack_id()),
                        ("model", Shape::NonEmptyStr),
                        ("author_model", Shape::NonEmptyStr),
                    ],
                    &[],
                ),
            ),
        ]
        .into_iter()
        .chain(settings_fields())
        .collect(),
        &["schema_version"],
    )
}

/// The shape of the `slopvac:` block of a document's front matter.
#[must_use]
pub fn block_shape() -> Shape {
    scoped(
        "front matter",
        vec![
            ("v", Shape::Version),
            ("genre", reference()),
            ("profile", profile()),
            ("locale", choice(LOCALES)),
            ("thresholds", thresholds()),
            ("categories", keyed(category_settings())),
            ("rules", keyed(rule_level())),
            ("terms", table(vec![("allow", strings())], &[])),
        ],
        &["v"],
    )
}

struct Ctx<'a> {
    text: &'a str,
    out: Vec<Diagnostic>,
}

impl Ctx<'_> {
    fn push(
        &mut self,
        code: Code,
        span: Range<usize>,
        path: &[String],
        message: String,
        label: String,
        help: Option<String>,
    ) {
        self.out.push(Diagnostic {
            code,
            message,
            label,
            span,
            help,
            path: path.to_vec(),
        });
    }

    fn type_error(&mut self, node: &Node, path: &[String], expected: &str, help: Option<String>) {
        let found = node.kind();
        self.push(
            Code::InvalidType,
            node.span.clone(),
            path,
            format!("expected {expected}, found {found}"),
            format!("{found} here"),
            help,
        );
    }

    fn value_error(
        &mut self,
        node: &Node,
        path: &[String],
        message: String,
        label: &str,
        help: Option<String>,
    ) {
        self.push(
            Code::InvalidValue,
            node.span.clone(),
            path,
            message,
            label.to_owned(),
            help,
        );
    }

    /// The first line of a node, so a missing key points at a header and not at a whole file.
    fn head(&self, node: &Node) -> Range<usize> {
        let start = node.span.start.min(self.text.len());
        let end = self.text[start..]
            .find('\n')
            .map_or(node.span.end, |i| start + i)
            .min(node.span.end.max(start));
        start..end
    }
}

fn child(path: &[String], seg: &str) -> Vec<String> {
    let mut p = path.to_vec();
    p.push(seg.to_owned());
    p
}

fn check(ctx: &mut Ctx<'_>, node: &Node, shape: &Shape, path: &[String]) {
    match shape {
        Shape::NonEmptyStr => match &node.value {
            Value::Str(s) if s.is_empty() => {
                ctx.value_error(
                    node,
                    path,
                    "the string is empty".into(),
                    "empty string",
                    None,
                );
            }
            Value::Str(_) => {}
            _ => ctx.type_error(node, path, "a string", None),
        },
        Shape::Pattern { name, check } => match &node.value {
            Value::Str(s) if check(s) => {}
            Value::Str(s) => ctx.value_error(
                node,
                path,
                format!("`{s}` is not a valid {name}"),
                "not valid here",
                None,
            ),
            _ => ctx.type_error(node, path, "a string", None),
        },
        Shape::Bool => {
            if !matches!(node.value, Value::Bool(_)) {
                ctx.type_error(node, path, "a boolean", None);
            }
        }
        Shape::Int { min, max } => check_int(ctx, node, *min, *max, path),
        Shape::Budget => match &node.value {
            Value::Str(s) if s == "none" => {}
            Value::Str(s) => ctx.value_error(
                node,
                path,
                format!("`{s}` is not a budget"),
                "not accepted",
                Some("use a number, or `none` to clear the budget".into()),
            ),
            _ => check_num(ctx, node, Some(0.0), None, path),
        },
        Shape::Num { min, max } => check_num(ctx, node, *min, *max, path),
        Shape::Choice { values, table_help } => check_choice(ctx, node, values, *table_help, path),
        Shape::Version => match node.value {
            Value::Int(3) => {}
            Value::Int(n) if n > 3 => ctx.value_error(
                node,
                path,
                format!("version {n} is newer than this slopvac reads"),
                "this build reads version 3",
                Some("upgrade slopvac".into()),
            ),
            Value::Int(n) => ctx.value_error(
                node,
                path,
                format!("version {n} is not supported; this build reads version 3"),
                "must be 3",
                Some("run `slopvac migrate` for a 2.x config".into()),
            ),
            _ => ctx.type_error(node, path, "the integer 3", None),
        },
        Shape::List {
            item,
            min_items,
            unique,
        } => check_list(ctx, node, item, *min_items, *unique, path),
        Shape::Table(t) => check_table(ctx, node, t, path),
        Shape::Map {
            key,
            key_name,
            value,
        } => match &node.value {
            Value::Table(entries) => {
                for e in entries {
                    let p = child(path, &e.key);
                    if !key(&e.key) {
                        ctx.push(
                            Code::InvalidValue,
                            e.key_span.clone(),
                            &p,
                            format!("`{}` is not a valid {key_name}", e.key),
                            "not valid here".into(),
                            Some("use `<pack>:<id>` with a lowercase pack, or a bare name".into()),
                        );
                    }
                    check(ctx, &e.node, value, &p);
                }
            }
            _ => ctx.type_error(node, path, "a table", None),
        },
        Shape::Either { scalar, table } => {
            if matches!(node.value, Value::Table(_)) {
                check(ctx, node, table, path);
            } else {
                check(ctx, node, scalar, path);
            }
        }
    }
}

#[allow(clippy::cast_precision_loss)]
fn as_f64(i: i64) -> f64 {
    i as f64
}

fn check_int(ctx: &mut Ctx<'_>, node: &Node, min: Option<i64>, max: Option<i64>, path: &[String]) {
    let Value::Int(i) = node.value else {
        ctx.type_error(node, path, "an integer", None);
        return;
    };
    if min.is_some_and(|m| i < m) || max.is_some_and(|m| i > m) {
        ctx.value_error(
            node,
            path,
            format!("{i} is out of range"),
            &range_label(min.map(as_f64), max.map(as_f64)),
            None,
        );
    }
}

fn check_num(ctx: &mut Ctx<'_>, node: &Node, min: Option<f64>, max: Option<f64>, path: &[String]) {
    let n = match node.value {
        Value::Int(i) => as_f64(i),
        Value::Float(f) => f,
        _ => {
            ctx.type_error(node, path, "a number", None);
            return;
        }
    };
    if !n.is_finite() {
        ctx.value_error(
            node,
            path,
            format!("{n} is not a finite number"),
            "must be finite",
            None,
        );
    } else if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
        ctx.value_error(
            node,
            path,
            format!("{n} is out of range"),
            &range_label(min, max),
            None,
        );
    }
}

fn check_choice(
    ctx: &mut Ctx<'_>,
    node: &Node,
    values: &[&str],
    table_help: Option<&str>,
    path: &[String],
) {
    let expected = format!("one of {}", join_ticks(values));
    match &node.value {
        Value::Str(s) if values.contains(&s.as_str()) => {}
        Value::Str(s) => {
            let help = closest(s, values.iter().copied()).map_or_else(
                || format!("use {expected}"),
                |c| format!("did you mean `{c}`?"),
            );
            ctx.value_error(
                node,
                path,
                format!("`{s}` is not an accepted value"),
                "not accepted",
                Some(help),
            );
        }
        Value::Table(_) => ctx.type_error(node, path, &expected, table_help.map(str::to_owned)),
        _ => ctx.type_error(node, path, &expected, None),
    }
}

fn check_list(
    ctx: &mut Ctx<'_>,
    node: &Node,
    item: &Shape,
    min_items: usize,
    unique: bool,
    path: &[String],
) {
    let Value::Array(items) = &node.value else {
        ctx.type_error(node, path, "an array", None);
        return;
    };
    if items.len() < min_items {
        ctx.value_error(
            node,
            path,
            format!("the list needs at least {min_items} item"),
            "empty list",
            None,
        );
    }
    for (i, it) in items.iter().enumerate() {
        check(ctx, it, item, &child(path, &i.to_string()));
        if unique && items[..i].iter().any(|prev| prev.to_json() == it.to_json()) {
            ctx.value_error(
                it,
                &child(path, &i.to_string()),
                "the list repeats an entry".into(),
                "repeated",
                None,
            );
        }
    }
}

fn check_table(ctx: &mut Ctx<'_>, node: &Node, shape: &TableShape, path: &[String]) {
    let Value::Table(entries) = &node.value else {
        ctx.type_error(node, path, "a table", None);
        return;
    };
    for e in entries {
        check_entry(ctx, e, shape, path);
    }
    for req in &shape.required {
        if node.entry(req).is_none() {
            ctx.push(
                Code::MissingKey,
                ctx.head(node),
                path,
                format!("missing required key `{req}`"),
                format!("`{req}` is required here"),
                None,
            );
        }
    }
}

fn check_entry(ctx: &mut Ctx<'_>, e: &Entry, shape: &TableShape, path: &[String]) {
    let p = child(path, &e.key);
    if let Some((_, field)) = shape.fields.iter().find(|(name, _)| *name == e.key) {
        check(ctx, &e.node, field, &p);
        return;
    }
    if e.key.starts_with("x-") {
        return;
    }
    let (message, help) = match shape.scope {
        Some(scope) if PROJECT_WIDE.contains(&e.key.as_str()) => (
            format!(
                "`{}` applies to the whole run and cannot be set in {scope}",
                e.key
            ),
            Some(format!("set `{}` in the project config", e.key)),
        ),
        _ => (
            format!("unknown key `{}`", e.key),
            closest(&e.key, shape.fields.iter().map(|(n, _)| *n))
                .map(|c| format!("did you mean `{c}`?")),
        ),
    };
    ctx.push(
        Code::UnknownKey,
        e.key_span.clone(),
        &p,
        message,
        "not a known key".into(),
        help,
    );
}

fn join_ticks(values: &[&str]) -> String {
    values
        .iter()
        .map(|v| format!("`{v}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn range_label(min: Option<f64>, max: Option<f64>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => format!("must be between {lo} and {hi}"),
        (Some(lo), None) => format!("must be at least {lo}"),
        (None, Some(hi)) => format!("must be at most {hi}"),
        (None, None) => "out of range".into(),
    }
}

/// Check a parsed `slopvac.toml` against the config schema.
#[must_use]
pub fn validate_config(root: &Node, text: &str) -> Vec<Diagnostic> {
    let mut ctx = Ctx {
        text,
        out: Vec::new(),
    };
    check(&mut ctx, root, &config_shape(), &[]);
    ctx.out
}

/// Check the `slopvac:` block of a parsed front-matter mapping.
///
/// The front matter must be a mapping, so a sequence, a scalar or an empty document is an
/// error at the root. A mapping without a `slopvac` key has nothing to check. A caller that
/// treats an empty front-matter block as absent skips it before calling this.
#[must_use]
pub fn validate_frontmatter(root: &Node, text: &str) -> Vec<Diagnostic> {
    let mut ctx = Ctx {
        text,
        out: Vec::new(),
    };
    if !matches!(root.value, Value::Table(_)) {
        let mut shown = root.clone();
        if shown.span.is_empty() {
            shown.span = 0..text.find('\n').unwrap_or(text.len());
        }
        ctx.type_error(&shown, &[], "a mapping", None);
        return ctx.out;
    }
    let Some(block) = root.get("slopvac") else {
        return Vec::new();
    };
    check(&mut ctx, block, &block_shape(), &["slopvac".to_owned()]);
    ctx.out
}
