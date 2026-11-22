//! The `slopvac-plugin.toml` manifest.
//!
//! The reader follows `schemas/v3/plugin-manifest.schema.json` and `docs/contract/plugins.md`.
//! Every error carries a JSON Pointer to the offending value. A missing key and an unknown key
//! point at the table that holds them, as the conformance corpus expects.

use std::fmt;
use std::io::Read;

use semver::{Version, VersionReq};
use slopvac_model::{ApiVersion, PackId};
use toml::{Table, Value};

/// Largest manifest the reader accepts, in bytes.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

type Result<T> = std::result::Result<T, ManifestError>;

/// Why a manifest was refused. Text taken from the file is kept raw and escaped once, in
/// `Display`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    /// The manifest breaks the contract at `pointer`.
    #[error("{}: {}", pointer_text(.pointer), slopvac_sanitize::line(.message))]
    Invalid {
        /// JSON Pointer to the offending value; empty for the whole file.
        pointer: String,
        /// What is wrong.
        message: String,
    },
    /// The manifest could not be read.
    #[error("cannot read the manifest")]
    Read(#[from] slopvac_fs::Error),
}

impl ManifestError {
    /// The JSON Pointer of an invalid value, if the manifest was read.
    #[must_use]
    pub fn pointer(&self) -> Option<&str> {
        match self {
            Self::Invalid { pointer, .. } => Some(pointer),
            Self::Read(_) => None,
        }
    }
}

/// The reason a TOML syntax error gives, without the text that caused it: the parser's own
/// message can quote the input.
fn toml_message(e: &toml::de::Error) -> String {
    e.span().map_or_else(
        || "not valid TOML".to_owned(),
        |span| format!("not valid TOML at byte {}", span.start),
    )
}

fn pointer_text(pointer: &str) -> String {
    if pointer.is_empty() {
        "/".to_owned()
    } else {
        slopvac_sanitize::line(pointer)
    }
}

fn err(pointer: &str, message: impl Into<String>) -> ManifestError {
    ManifestError::Invalid {
        pointer: pointer.to_owned(),
        message: message.into(),
    }
}

/// What a pack is: data only, or data plus a plugin command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PackKind {
    /// Rules, styles, genres, terms and taggers. No code.
    Data,
    /// Adds a command for the host to run.
    Code,
}

/// A path inside a pack: relative, with forward slashes and no `..` segment. A glob is allowed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PackPath(String);

impl PackPath {
    /// The path as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn parse(s: &str) -> std::result::Result<Self, String> {
        let drive = s
            .as_bytes()
            .get(..2)
            .is_some_and(|b| b[0].is_ascii_alphabetic() && b[1] == b':');
        let reason = if s.is_empty() {
            "a pack path must not be empty"
        } else if s.starts_with('/') {
            "a pack path must be relative"
        } else if drive {
            "a pack path must not start with a drive letter"
        } else if s.contains('\\') {
            "a pack path must use forward slashes"
        } else if s.split('/').any(|segment| segment == "..") {
            "a pack path must not leave the pack with a `..` segment"
        } else {
            return Ok(Self(s.to_owned()));
        };
        Err(reason.to_owned())
    }
}

impl fmt::Display for PackPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&slopvac_sanitize::line(&self.0))
    }
}

/// The files a pack contributes, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Provides {
    /// Category files.
    pub categories: Vec<PackPath>,
    /// Style files.
    pub styles: Vec<PackPath>,
    /// Genre files.
    pub genres: Vec<PackPath>,
    /// Terms files.
    pub terms: Vec<PackPath>,
    /// Tagger files.
    pub taggers: Vec<PackPath>,
    /// Config files a project may extend.
    pub configs: Vec<PackPath>,
    /// Markdown snippets for `slopvac prime`.
    pub prime: Vec<PackPath>,
    /// Blocklist files.
    pub blocklists: Vec<PackPath>,
}

/// A pack this pack needs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Requirement {
    /// The required pack.
    pub id: PackId,
    /// The versions that satisfy the requirement.
    pub version: VersionReq,
}

impl Requirement {
    /// Require `id` at the versions `version` allows.
    #[must_use]
    pub fn new(id: PackId, version: VersionReq) -> Self {
        Self { id, version }
    }
}

/// One hook a plugin may offer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct HookDecl {
    /// Hook kind, such as `tagger`. The form is checked; unknown kinds pass.
    pub kind: String,
    /// Plugin-local hook id.
    pub id: String,
}

impl HookDecl {
    /// Declare a hook of `kind` with the plugin-local `id`.
    #[must_use]
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

/// How the host runs a plugin's `command`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PluginRuntime {
    /// A program from the pack, or a bare name the plugin's own environment provides.
    #[default]
    Native,
    /// A script of the pack, run by a managed Python interpreter.
    Python,
}

/// The `[plugin]` table: how to run the plugin and its pre-spawn declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PluginTable {
    /// Executable inside the pack, or a bare name. With the Python runtime, a script inside the
    /// pack that does not start with a dash.
    pub command: String,
    /// How the host runs `command`.
    pub runtime: PluginRuntime,
    /// The Python version request for the managed environment. Set exactly when `runtime` is
    /// [`PluginRuntime::Python`].
    pub python: Option<String>,
    /// Arguments.
    pub args: Vec<String>,
    /// Environment variable names the plugin may receive.
    pub env: Vec<String>,
    /// Whether the plugin may open connections.
    pub network: bool,
    /// The hooks the plugin may offer.
    pub hooks: Vec<HookDecl>,
    /// Features the plugin requires.
    pub features_required: Vec<String>,
    /// Features the plugin may use.
    pub features_optional: Vec<String>,
}

impl PluginTable {
    /// A plugin table that runs `command` with no arguments, environment, hooks or features. Set
    /// the other public fields to fill it in.
    #[must_use]
    pub fn new(command: impl Into<String>, network: bool) -> Self {
        Self {
            command: command.into(),
            runtime: PluginRuntime::Native,
            python: None,
            args: Vec::new(),
            env: Vec::new(),
            network,
            hooks: Vec::new(),
            features_required: Vec::new(),
            features_optional: Vec::new(),
        }
    }
}

/// A parsed `slopvac-plugin.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Manifest {
    /// Pack id, the prefix of every rule id the pack provides.
    pub id: PackId,
    /// Pack version.
    pub version: Version,
    /// Extension API the pack targets.
    pub slopvac_api: ApiVersion,
    /// Data or plugin pack.
    pub kind: PackKind,
    /// One-line description.
    pub description: Option<String>,
    /// SPDX license expression.
    pub license: Option<String>,
    /// Home page URI.
    pub homepage: Option<String>,
    /// Files the pack contributes.
    pub provides: Provides,
    /// Packs this pack needs.
    pub requires: Vec<Requirement>,
    /// The plugin table; present exactly when `kind` is [`PackKind::Code`].
    pub plugin: Option<PluginTable>,
}

impl Manifest {
    /// Parse manifest text. The id `core` is reserved for the built-in plugin and is refused.
    pub fn parse(text: &str) -> Result<Self> {
        Self::parse_with(text, false)
    }

    /// Parse manifest text, accepting the reserved id `core`. Only the loader calls this, and it
    /// accepts the manifest only for the built-in core plugin.
    pub(crate) fn parse_allowing_core(text: &str) -> Result<Self> {
        Self::parse_with(text, true)
    }

    fn parse_with(text: &str, allow_core: bool) -> Result<Self> {
        if u64::try_from(text.len()).map_or(true, |n| n > MAX_MANIFEST_BYTES) {
            return Err(err("", format!("larger than {MAX_MANIFEST_BYTES} bytes")));
        }
        let root: Value = toml::from_str(text).map_err(|e| err("", toml_message(&e)))?;
        parse_root(&root, allow_core)
    }

    /// Read a manifest from `reader`, refusing more than 256 KiB.
    pub fn from_reader(reader: impl Read) -> Result<Self> {
        let bytes = slopvac_fs::read_capped(reader, MAX_MANIFEST_BYTES)?;
        let text = String::from_utf8(bytes).map_err(|_| err("", "not valid UTF-8"))?;
        Self::parse(&text)
    }
}

/// A TOML table with its JSON Pointer, read under the unknown-key policy: a key outside
/// `allowed` is an error unless it starts with `x-`.
struct Obj<'a> {
    table: &'a Table,
    pointer: String,
}

impl<'a> Obj<'a> {
    fn new(value: &'a Value, pointer: String, allowed: &[&str]) -> Result<Self> {
        let table = value
            .as_table()
            .ok_or_else(|| err(&pointer, "must be a table"))?;
        if table
            .keys()
            .any(|k| !k.starts_with("x-") && !allowed.contains(&k.as_str()))
        {
            return Err(err(&pointer, "unknown key"));
        }
        Ok(Self { table, pointer })
    }

    fn at(&self, key: &str) -> String {
        format!("{}/{key}", self.pointer)
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.table.get(key)
    }

    fn required(&self, key: &str) -> Result<&'a Value> {
        self.get(key)
            .ok_or_else(|| err(&self.pointer, format!("missing required key `{key}`")))
    }

    fn string(&self, key: &str) -> Result<&'a str> {
        self.required(key)?
            .as_str()
            .ok_or_else(|| err(&self.at(key), "must be a string"))
    }

    fn optional_string(&self, key: &str) -> Result<Option<&'a str>> {
        self.get(key)
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| err(&self.at(key), "must be a string"))
            })
            .transpose()
    }

    fn non_empty(&self, key: &str) -> Result<Option<String>> {
        match self.optional_string(key)? {
            Some("") => Err(err(&self.at(key), "must not be empty")),
            other => Ok(other.map(str::to_owned)),
        }
    }

    /// An optional array of strings, each checked by `check`, with no repeats.
    fn strings<T: Eq + Clone>(
        &self,
        key: &str,
        check: impl Fn(&str) -> std::result::Result<T, String>,
    ) -> Result<Vec<T>> {
        let Some(value) = self.get(key) else {
            return Ok(Vec::new());
        };
        let items = value
            .as_array()
            .ok_or_else(|| err(&self.at(key), "must be an array"))?;
        let mut out: Vec<T> = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let at = format!("{}/{i}", self.at(key));
            let text = item.as_str().ok_or_else(|| err(&at, "must be a string"))?;
            let parsed = check(text).map_err(|m| err(&at, m))?;
            if out.contains(&parsed) {
                return Err(err(&at, "repeats an earlier item"));
            }
            out.push(parsed);
        }
        Ok(out)
    }
}

fn parse_root(value: &Value, allow_core: bool) -> Result<Manifest> {
    let root = Obj::new(
        value,
        String::new(),
        &[
            "schema_version",
            "id",
            "version",
            "slopvac_api",
            "kind",
            "description",
            "license",
            "homepage",
            "provides",
            "requires",
            "plugin",
        ],
    )?;
    match root.required("schema_version")?.as_integer() {
        Some(3) => {}
        _ => return Err(err("/schema_version", "must be 3")),
    }
    let id = pack_id(root.string("id")?).map_err(|m| err("/id", m))?;
    if id.as_str() == "core" && !allow_core {
        return Err(err("/id", "`core` is reserved for the built-in plugin"));
    }
    let version = parse_version(root.string("version")?).map_err(|m| err("/version", m))?;
    let slopvac_api = root
        .required("slopvac_api")?
        .as_integer()
        .and_then(|n| u32::try_from(n).ok())
        .and_then(|n| ApiVersion::new(n).ok())
        .ok_or_else(|| err("/slopvac_api", "must be an integer of at least 1"))?;
    let kind = match root.string("kind")? {
        "data" => PackKind::Data,
        "code" => PackKind::Code,
        _ => {
            return Err(err("/kind", "not a plugin kind (`data` or `code`)"));
        }
    };
    let description = root.non_empty("description")?;
    let license = root.non_empty("license")?;
    let homepage = root.non_empty("homepage")?;
    if let Some(h) = &homepage {
        check_uri(h).map_err(|m| err("/homepage", m))?;
    }
    let provides = root
        .get("provides")
        .map(|v| parse_provides(v, "/provides".to_owned()))
        .transpose()?
        .unwrap_or_default();
    let requires = parse_requires(&root)?;
    let plugin = root
        .get("plugin")
        .map(|v| parse_plugin(v, "/plugin".to_owned()))
        .transpose()?;
    match (kind, &plugin) {
        (PackKind::Code, None) => {
            return Err(err("", "a plugin pack needs a `[plugin]` table"));
        }
        (PackKind::Data, Some(_)) => {
            return Err(err("", "a data pack must not have a `[plugin]` table"));
        }
        _ => {}
    }
    Ok(Manifest {
        id,
        version,
        slopvac_api,
        kind,
        description,
        license,
        homepage,
        provides,
        requires,
        plugin,
    })
}

fn pack_id(s: &str) -> std::result::Result<PackId, String> {
    PackId::new(s).map_err(|_| {
        "not a pack id (lowercase letters, digits and `-`, starting with a letter)".to_owned()
    })
}

/// Semantic version 2.0.0 without build metadata.
fn parse_version(s: &str) -> std::result::Result<Version, String> {
    if s.contains('+') {
        return Err("a pack version must not carry build metadata".to_owned());
    }
    Version::parse(s).map_err(|_| "not a semantic version".to_owned())
}

/// One or more comparators: an optional operator and a version of one to three numeric parts. A
/// pre-release follows a full `MAJOR.MINOR.PATCH` only. Spaces are allowed after an operator and
/// around a comma, and nowhere else.
fn parse_requirement(s: &str) -> std::result::Result<VersionReq, String> {
    let bad = || "not a version requirement".to_owned();
    if s.starts_with(' ') || s.ends_with(' ') {
        return Err(bad());
    }
    for comparator in s.split(',') {
        let c = comparator.trim_matches(' ');
        let c = ["<=", ">=", "<", ">", "=", "^", "~"]
            .iter()
            .find_map(|op| c.strip_prefix(op))
            .unwrap_or(c)
            .trim_start_matches(' ');
        let (numbers, pre) = match c.split_once('-') {
            Some((n, p)) => (n, Some(p)),
            None => (c, None),
        };
        let parts: Vec<&str> = numbers.split('.').collect();
        let numeric = |p: &&str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
        let pre_ok = pre.is_none_or(|p| {
            parts.len() == 3
                && !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'))
        });
        if !(1..=3).contains(&parts.len()) || !parts.iter().all(numeric) || !pre_ok {
            return Err(bad());
        }
    }
    VersionReq::parse(s).map_err(|_| bad())
}

/// A URI with a scheme, no whitespace or control characters and no user-info.
fn check_uri(s: &str) -> std::result::Result<(), String> {
    let mut chars = s.chars();
    let scheme_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars
            .by_ref()
            .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')))
            == Some(':');
    if !scheme_ok || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("not a URI".to_owned());
    }
    // A manifest is committed and printed, so it carries no credentials.
    let authority = s.split_once("//").map_or("", |(_, rest)| rest);
    let authority = authority.split(['/', '?', '#']).next().unwrap_or("");
    if authority.contains('@') {
        return Err("a URI must not carry user-info (credentials)".to_owned());
    }
    Ok(())
}

fn parse_provides(value: &Value, pointer: String) -> Result<Provides> {
    let obj = Obj::new(
        value,
        pointer,
        &[
            "categories",
            "styles",
            "genres",
            "terms",
            "taggers",
            "configs",
            "prime",
            "blocklists",
        ],
    )?;
    let list = |key| obj.strings(key, PackPath::parse);
    Ok(Provides {
        categories: list("categories")?,
        styles: list("styles")?,
        genres: list("genres")?,
        terms: list("terms")?,
        taggers: list("taggers")?,
        configs: list("configs")?,
        prime: list("prime")?,
        blocklists: list("blocklists")?,
    })
}

fn parse_requires(root: &Obj<'_>) -> Result<Vec<Requirement>> {
    let Some(value) = root.get("requires") else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| err("/requires", "must be an array"))?;
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let obj = Obj::new(item, format!("/requires/{i}"), &["id", "version"])?;
            let id = pack_id(obj.string("id")?).map_err(|m| err(&obj.at("id"), m))?;
            let version = parse_requirement(obj.string("version")?)
                .map_err(|m| err(&obj.at("version"), m))?;
            Ok(Requirement { id, version })
        })
        .collect()
}

/// `^[a-z][a-z0-9-]*$`
fn slug(s: &str) -> std::result::Result<String, String> {
    let mut chars = s.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(s.to_owned())
    } else {
        Err("not a lowercase name".to_owned())
    }
}

/// A feature name: a lowercase name, or an `x-` private extension.
fn feature(s: &str) -> std::result::Result<String, String> {
    let private = s.strip_prefix("x-").is_some_and(|r| {
        !r.is_empty()
            && r.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    });
    if private { Ok(s.to_owned()) } else { slug(s) }
}

/// `^[A-Z_][A-Z0-9_]*$`
fn env_name(s: &str) -> std::result::Result<String, String> {
    let mut chars = s.chars();
    let ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(s.to_owned())
    } else {
        Err("not an environment variable name".to_owned())
    }
}

/// The longest `python` request the schema takes.
const PYTHON_REQUEST_MAX: usize = 64;

/// Read `runtime` and `python`. The Python runtime needs a `python` request and a script path
/// that no interpreter option can imitate; the native runtime takes no `python`.
fn parse_runtime(obj: &Obj<'_>, command: &str) -> Result<(PluginRuntime, Option<String>)> {
    let runtime = match obj.optional_string("runtime")? {
        None | Some("native") => PluginRuntime::Native,
        Some("python") => PluginRuntime::Python,
        Some(_) => {
            return Err(err(&obj.at("runtime"), "must be `native` or `python`"));
        }
    };
    let python = obj.optional_string("python")?;
    match (runtime, python) {
        (PluginRuntime::Native, None) => Ok((runtime, None)),
        (PluginRuntime::Native, Some(_)) => {
            Err(err(&obj.pointer, "`python` needs `runtime = \"python\"`"))
        }
        (PluginRuntime::Python, None) => Err(err(
            &obj.pointer,
            "`runtime = \"python\"` needs a `python` version request",
        )),
        (PluginRuntime::Python, Some(request)) => {
            if request.len() > PYTHON_REQUEST_MAX || request.chars().any(char::is_whitespace) {
                return Err(err(
                    &obj.at("python"),
                    "must be at most 64 characters with no whitespace",
                ));
            }
            parse_requirement(request).map_err(|m| err(&obj.at("python"), m))?;
            PackPath::parse(command).map_err(|m| err(&obj.at("command"), m))?;
            if command.starts_with('-') {
                return Err(err(
                    &obj.at("command"),
                    "a script path must not start with a dash",
                ));
            }
            Ok((runtime, Some(request.to_owned())))
        }
    }
}

fn parse_plugin(value: &Value, pointer: String) -> Result<PluginTable> {
    let obj = Obj::new(
        value,
        pointer,
        &[
            "command", "runtime", "python", "args", "env", "network", "hooks", "features",
        ],
    )?;
    let command = obj.string("command")?;
    if command.is_empty() {
        return Err(err(&obj.at("command"), "must not be empty"));
    }
    let (runtime, python) = parse_runtime(&obj, command)?;
    let network = obj
        .required("network")?
        .as_bool()
        .ok_or_else(|| err(&obj.at("network"), "must be true or false"))?;
    let hooks_value = obj.required("hooks")?;
    let features_value = obj.required("features")?;
    let args = match obj.get("args") {
        None => Vec::new(),
        Some(v) => v
            .as_array()
            .ok_or_else(|| err(&obj.at("args"), "must be an array"))?
            .iter()
            .enumerate()
            .map(|(i, a)| {
                a.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| err(&format!("{}/{i}", obj.at("args")), "must be a string"))
            })
            .collect::<Result<_>>()?,
    };
    let env = obj.strings("env", env_name)?;
    let hooks = hooks_value
        .as_array()
        .ok_or_else(|| err(&obj.at("hooks"), "must be an array"))?
        .iter()
        .enumerate()
        .map(|(i, hook)| {
            let h = Obj::new(hook, format!("{}/{i}", obj.at("hooks")), &["kind", "id"])?;
            let kind = slug(h.string("kind")?).map_err(|m| err(&h.at("kind"), m))?;
            let id = slug(h.string("id")?).map_err(|m| err(&h.at("id"), m))?;
            Ok(HookDecl { kind, id })
        })
        .collect::<Result<Vec<_>>>()?;
    let features = Obj::new(
        features_value,
        obj.at("features"),
        &["required", "optional"],
    )?;
    features.required("required")?;
    features.required("optional")?;
    Ok(PluginTable {
        command: command.to_owned(),
        runtime,
        python,
        args,
        env,
        network,
        hooks,
        features_required: features.strings("required", feature)?,
        features_optional: features.strings("optional", feature)?,
    })
}
