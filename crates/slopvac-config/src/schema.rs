//! Structural validation of `slopvac.toml` and the front-matter `slopvac:` block.
//!
//! The shapes mirror `schemas/v3/config.schema.json`, `schemas/v3/frontmatter.schema.json`
//! and `schemas/v3/style.schema.json`. The conformance corpus under
//! `schemas/v3/conformance/` is the shared test of both.

use std::ops::Range;

use crate::diag::{Code, Diagnostic, closest};
use crate::messages::{ConfigPhrase, ConfigText};
use crate::tree::{Entry, Node, Value};

/// The shape a value must have.
#[derive(Debug, Clone)]
pub enum Shape {
    /// A string with at least one character.
    NonEmptyStr,
    /// A string that `check` accepts. `name` completes "a valid ...".
    Pattern {
        /// Name of the accepted form, such as "plugin id".
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
        /// A key of table items that must differ between items, such as `id`.
        unique_key: Option<&'static str>,
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
        unique_key: None,
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
        name: ConfigPhrase::NounPackId.as_static(),
        check: is_pack_id,
    }
}

/// The id of the built-in plugin. It is compiled into the binary, so no config entry may name it.
const BUILT_IN_PLUGIN: &str = "core";

fn is_plugin_id(s: &str) -> bool {
    is_pack_id(s) && s != BUILT_IN_PLUGIN
}

fn plugin_id() -> Shape {
    Shape::Pattern {
        name: ConfigPhrase::NounPluginId.as_static(),
        check: is_plugin_id,
    }
}

fn reference() -> Shape {
    Shape::Pattern {
        name: ConfigPhrase::NounReference.as_static(),
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
        table_help: Some(ConfigPhrase::HelpBareLevel.as_static()),
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
        key_name: ConfigPhrase::NounRuleKey.as_static(),
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
        unique_key: None,
    }
}

fn source() -> Shape {
    table(
        vec![
            ("id", plugin_id()),
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
            (
                "plugins",
                Shape::List {
                    item: Box::new(source()),
                    min_items: 0,
                    unique: false,
                    unique_key: Some("id"),
                },
            ),
            (
                "genres",
                list(scoped(
                    ConfigPhrase::ScopeGenreBlock.as_static(),
                    genre_fields,
                    &["genre", "files"],
                )),
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
                                unique_key: None,
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
                list(scoped(
                    ConfigPhrase::ScopeOverride.as_static(),
                    override_fields,
                    &["files"],
                )),
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

fn is_rel_path(s: &str) -> bool {
    let drive = s.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && s.as_bytes().get(1) == Some(&b':');
    !s.is_empty()
        && !s.starts_with(['/', '\\'])
        && !drive
        && !s.contains('\\')
        && !s.split('/').any(|seg| seg == "..")
}

/// The shape of a style preset file.
#[must_use]
pub fn style_shape() -> Shape {
    table(
        vec![
            ("schema_version", Shape::Version),
            ("description", Shape::NonEmptyStr),
            (
                "extends",
                Shape::List {
                    item: Box::new(reference()),
                    min_items: 0,
                    unique: true,
                    unique_key: None,
                },
            ),
            (
                "conflicts",
                Shape::List {
                    item: Box::new(reference()),
                    min_items: 0,
                    unique: true,
                    unique_key: None,
                },
            ),
            ("categories", keyed(category_settings())),
            ("rules", keyed(rule_level())),
            (
                "terms",
                table(
                    vec![
                        ("allow", strings()),
                        (
                            "blocklists",
                            list(Shape::Pattern {
                                name: ConfigPhrase::NounRelativePath.as_static(),
                                check: is_rel_path,
                            }),
                        ),
                    ],
                    &[],
                ),
            ),
        ],
        &["schema_version"],
    )
}

/// The shape of the `slopvac:` block of a document's front matter.
#[must_use]
pub fn block_shape() -> Shape {
    scoped(
        ConfigPhrase::ScopeFrontMatter.as_static(),
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

/// The shape of an inline directive: the front-matter fields, with no version.
#[must_use]
pub fn directive_shape() -> Shape {
    let Shape::Table(mut t) = block_shape() else {
        unreachable!("the block shape is a table");
    };
    t.fields.retain(|(name, _)| *name != "v");
    t.required.clear();
    t.scope = Some(ConfigPhrase::ScopeDirective.as_static());
    Shape::Table(t)
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
            data: None,
        });
    }

    fn type_error(&mut self, node: &Node, path: &[String], expected: &str, help: Option<String>) {
        let found = node.kind();
        self.push(
            Code::InvalidType,
            node.span.clone(),
            path,
            ConfigText::TypeMismatch {
                expected: expected.to_owned(),
                found: found.to_owned(),
            }
            .to_string(),
            ConfigText::FoundHere {
                found: found.to_owned(),
            }
            .to_string(),
            help,
        );
    }

    fn value_error(
        &mut self,
        node: &Node,
        path: &[String],
        message: String,
        label: String,
        help: Option<String>,
    ) {
        self.push(
            Code::InvalidValue,
            node.span.clone(),
            path,
            message,
            label,
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
                    ConfigPhrase::EmptyString.text(),
                    ConfigPhrase::LabelEmptyString.text(),
                    None,
                );
            }
            Value::Str(_) => {}
            _ => ctx.type_error(node, path, ConfigPhrase::KindString.as_static(), None),
        },
        Shape::Pattern { name, check } => match &node.value {
            Value::Str(s) if check(s) => {}
            Value::Str(s) => ctx.value_error(
                node,
                path,
                ConfigText::NotValid {
                    value: s.clone(),
                    name: (*name).to_owned(),
                }
                .to_string(),
                ConfigPhrase::LabelNotValidHere.text(),
                None,
            ),
            _ => ctx.type_error(node, path, ConfigPhrase::KindString.as_static(), None),
        },
        Shape::Bool => {
            if !matches!(node.value, Value::Bool(_)) {
                ctx.type_error(node, path, ConfigPhrase::KindBoolean.as_static(), None);
            }
        }
        Shape::Int { min, max } => check_int(ctx, node, *min, *max, path),
        Shape::Budget => match &node.value {
            Value::Str(s) if s == "none" => {}
            Value::Str(s) => ctx.value_error(
                node,
                path,
                ConfigText::NotBudget { value: s.clone() }.to_string(),
                ConfigPhrase::LabelNotAccepted.text(),
                Some(ConfigPhrase::HelpBudget.text()),
            ),
            _ => check_num(ctx, node, Some(0.0), None, path),
        },
        Shape::Num { min, max } => check_num(ctx, node, *min, *max, path),
        Shape::Choice { values, table_help } => check_choice(ctx, node, values, *table_help, path),
        Shape::Version => check_version_value(ctx, node, path),
        Shape::List {
            item,
            min_items,
            unique,
            unique_key,
        } => check_list(ctx, node, item, *min_items, (*unique, *unique_key), path),
        Shape::Table(t) => check_table(ctx, node, t, path),
        Shape::Map {
            key,
            key_name,
            value,
        } => check_map(ctx, node, (*key, key_name), value, path),
        Shape::Either { scalar, table } => {
            if matches!(node.value, Value::Table(_)) {
                check(ctx, node, table, path);
            } else {
                check(ctx, node, scalar, path);
            }
        }
    }
}

fn check_map(
    ctx: &mut Ctx<'_>,
    node: &Node,
    (key, key_name): (fn(&str) -> bool, &str),
    value: &Shape,
    path: &[String],
) {
    let Value::Table(entries) = &node.value else {
        ctx.type_error(node, path, ConfigPhrase::KindTable.as_static(), None);
        return;
    };
    for e in entries {
        let p = child(path, &e.key);
        if !key(&e.key) {
            ctx.push(
                Code::InvalidValue,
                e.key_span.clone(),
                &p,
                ConfigText::NotValid {
                    value: e.key.clone(),
                    name: key_name.to_owned(),
                }
                .to_string(),
                ConfigPhrase::LabelNotValidHere.text(),
                Some(ConfigPhrase::HelpReferenceKey.text()),
            );
        }
        check(ctx, &e.node, value, &p);
    }
}

fn check_version_value(ctx: &mut Ctx<'_>, node: &Node, path: &[String]) {
    match node.value {
        Value::Int(3) => {}
        Value::Int(n) if n > 3 => ctx.value_error(
            node,
            path,
            ConfigText::VersionNewer {
                version: n.to_string(),
            }
            .to_string(),
            ConfigPhrase::LabelReadsVersion3.text(),
            Some(ConfigPhrase::HelpUpgrade.text()),
        ),
        Value::Int(n) => ctx.value_error(
            node,
            path,
            ConfigText::VersionUnsupported {
                version: n.to_string(),
            }
            .to_string(),
            ConfigPhrase::LabelMustBe3.text(),
            Some(ConfigPhrase::HelpMigrateVersion.text()),
        ),
        _ => ctx.type_error(node, path, ConfigPhrase::KindIntegerThree.as_static(), None),
    }
}

#[allow(clippy::cast_precision_loss)]
fn as_f64(i: i64) -> f64 {
    i as f64
}

fn check_int(ctx: &mut Ctx<'_>, node: &Node, min: Option<i64>, max: Option<i64>, path: &[String]) {
    let Value::Int(i) = node.value else {
        ctx.type_error(node, path, ConfigPhrase::KindInteger.as_static(), None);
        return;
    };
    if min.is_some_and(|m| i < m) || max.is_some_and(|m| i > m) {
        ctx.value_error(
            node,
            path,
            ConfigText::OutOfRange {
                value: i.to_string(),
            }
            .to_string(),
            range_label(min.map(as_f64), max.map(as_f64)),
            None,
        );
    }
}

fn check_num(ctx: &mut Ctx<'_>, node: &Node, min: Option<f64>, max: Option<f64>, path: &[String]) {
    let n = match node.value {
        Value::Int(i) => as_f64(i),
        Value::Float(f) => f,
        _ => {
            ctx.type_error(node, path, ConfigPhrase::KindNumber.as_static(), None);
            return;
        }
    };
    if !n.is_finite() {
        ctx.value_error(
            node,
            path,
            ConfigText::NotFinite {
                value: n.to_string(),
            }
            .to_string(),
            ConfigPhrase::LabelMustBeFinite.text(),
            None,
        );
    } else if min.is_some_and(|m| n < m) || max.is_some_and(|m| n > m) {
        ctx.value_error(
            node,
            path,
            ConfigText::OutOfRange {
                value: n.to_string(),
            }
            .to_string(),
            range_label(min, max),
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
    let expected = ConfigText::ExpectedOneOf {
        values: join_ticks(values),
    }
    .to_string();
    match &node.value {
        Value::Str(s) if values.contains(&s.as_str()) => {}
        Value::Str(s) => {
            let help = closest(s, values.iter().copied()).map_or_else(
                || {
                    ConfigText::UseOneOf {
                        expected: expected.clone(),
                    }
                    .to_string()
                },
                |c| {
                    ConfigText::DidYouMean {
                        suggestion: c.to_owned(),
                    }
                    .to_string()
                },
            );
            ctx.value_error(
                node,
                path,
                ConfigText::NotAccepted { value: s.clone() }.to_string(),
                ConfigPhrase::LabelNotAccepted.text(),
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
    (unique, unique_key): (bool, Option<&'static str>),
    path: &[String],
) {
    let Value::Array(items) = &node.value else {
        ctx.type_error(node, path, ConfigPhrase::KindArray.as_static(), None);
        return;
    };
    if items.len() < min_items {
        ctx.value_error(
            node,
            path,
            ConfigText::ListTooShort { min_items }.to_string(),
            ConfigPhrase::LabelEmptyList.text(),
            None,
        );
    }
    for (i, it) in items.iter().enumerate() {
        check(ctx, it, item, &child(path, &i.to_string()));
        if unique && items[..i].iter().any(|prev| prev.to_json() == it.to_json()) {
            ctx.value_error(
                it,
                &child(path, &i.to_string()),
                ConfigPhrase::ListRepeats.text(),
                ConfigPhrase::LabelRepeated.text(),
                None,
            );
        }
        if let Some(key) = unique_key {
            check_unique_key(ctx, &items[..i], it, key, &child(path, &i.to_string()));
        }
    }
}

/// Flags `item` when an earlier item holds the same string under `key`.
fn check_unique_key(ctx: &mut Ctx<'_>, earlier: &[Node], item: &Node, key: &str, path: &[String]) {
    let Some(value) = item.get(key) else { return };
    let Some(text) = value.as_str() else { return };
    if earlier
        .iter()
        .any(|prev| prev.get(key).and_then(Node::as_str) == Some(text))
    {
        ctx.value_error(
            value,
            &child(path, key),
            ConfigText::KeyRepeated {
                value: text.to_owned(),
                key: key.to_owned(),
            }
            .to_string(),
            ConfigPhrase::LabelRepeated.text(),
            Some(
                ConfigText::HelpRemoveRepeatedKey {
                    key: key.to_owned(),
                }
                .to_string(),
            ),
        );
    }
}

fn check_table(ctx: &mut Ctx<'_>, node: &Node, shape: &TableShape, path: &[String]) {
    let Value::Table(entries) = &node.value else {
        ctx.type_error(node, path, ConfigPhrase::KindTable.as_static(), None);
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
                ConfigText::MissingKeyMessage {
                    key: (*req).to_owned(),
                }
                .to_string(),
                ConfigText::MissingKeyLabel {
                    key: (*req).to_owned(),
                }
                .to_string(),
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
            ConfigText::ProjectWideKey {
                key: e.key.clone(),
                scope: scope.to_owned(),
            }
            .to_string(),
            Some(ConfigText::ProjectWideHelp { key: e.key.clone() }.to_string()),
        ),
        _ => (
            ConfigText::UnknownKeyMessage { key: e.key.clone() }.to_string(),
            closest(&e.key, shape.fields.iter().map(|(n, _)| *n)).map(|c| {
                ConfigText::DidYouMean {
                    suggestion: c.to_owned(),
                }
                .to_string()
            }),
        ),
    };
    ctx.push(
        Code::UnknownKey,
        e.key_span.clone(),
        &p,
        message,
        ConfigPhrase::LabelNotKnownKey.text(),
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
        (Some(lo), Some(hi)) => ConfigText::RangeBetween {
            min: lo.to_string(),
            max: hi.to_string(),
        }
        .to_string(),
        (Some(lo), None) => ConfigText::RangeAtLeast {
            min: lo.to_string(),
        }
        .to_string(),
        (None, Some(hi)) => ConfigText::RangeAtMost {
            max: hi.to_string(),
        }
        .to_string(),
        (None, None) => ConfigPhrase::LabelOutOfRange.text(),
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

/// Check a parsed style preset against the style schema.
#[must_use]
pub fn validate_style(root: &Node, text: &str) -> Vec<Diagnostic> {
    let mut ctx = Ctx {
        text,
        out: Vec::new(),
    };
    check(&mut ctx, root, &style_shape(), &[]);
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
        ctx.type_error(&shown, &[], ConfigPhrase::KindMapping.as_static(), None);
        return ctx.out;
    }
    let Some(block) = root.get("slopvac") else {
        return Vec::new();
    };
    check(&mut ctx, block, &block_shape(), &["slopvac".to_owned()]);
    ctx.out
}

/// Check the fields of one inline directive, parsed into a table.
#[must_use]
pub fn validate_directive(node: &Node, text: &str) -> Vec<Diagnostic> {
    let mut ctx = Ctx {
        text,
        out: Vec::new(),
    };
    check(
        &mut ctx,
        node,
        &directive_shape(),
        &["directive".to_owned()],
    );
    ctx.out
}
