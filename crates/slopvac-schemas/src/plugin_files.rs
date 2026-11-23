//! Schema-only types for the plugin index and the plugin lockfile.
//!
//! The readers in `slopvac-plugins` parse both documents by hand and have no type to derive a
//! schema from. These types describe the same documents so the generator can write
//! `plugin-index.schema.json` and `plugins-lock.schema.json`. No code reads a value of them. The
//! two-sided corpus under `schemas/v3/conformance` keeps them honest: each case runs through the
//! real reader and through the committed schema.

use schemars::{JsonSchema, Schema};
use serde_json::json;
use slopvac_model::{ApiVersion, PackId, SchemaVersion, absent_not_null, x_extensions};

use crate::artifacts::{PluginId, Semver};

/// The characters that count as white space in a source, spelled out because the engines read
/// `\s` differently. It is the union of what Python and ECMA-262 read as `\s`, so the pattern
/// refuses a character if either engine refuses it. That holds Unicode `White_Space`, the set the
/// readers' `char::is_whitespace` uses, plus U+001C to U+001F (Python) and U+FEFF (ECMA).
const WHITESPACE: &str =
    r"\x09-\x0D\x1C-\x20\x85\xA0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000\uFEFF";

/// The longest source the index reader accepts, in characters.
const INDEX_SOURCE_CHARS: u64 = 2048;
/// Most entries the index reader accepts.
const INDEX_ENTRIES: u64 = 100_000;
/// Longest pack id and namespace the index reader accepts, in characters.
const INDEX_ID_CHARS: u64 = 64;
/// Largest `slopvac_api` the index reader accepts.
const INDEX_API: u64 = 4_294_967_295;
/// Most packs the lock reader accepts.
const LOCK_PACKS: u64 = 1000;
/// Most `requires` entries the lock reader accepts in one pack.
const LOCK_REQUIRES: u64 = 256;
/// The lock reader's cap on a source, in bytes. A schema counts characters, so this is the
/// closest bound it can state.
const LOCK_SOURCE_CAP: u64 = 4096;

/// A pattern template with the white space class filled in.
fn with_whitespace(template: &str) -> String {
    template.replace("{WS}", WHITESPACE)
}

fn index_source(schema: &mut Schema) {
    // Authority: one or more characters that end at `/`, `?` or `#`, with no `@`. Then an
    // optional rest with no white space. A git source ends in `@` and a full commit hash.
    schema.insert(
        "pattern".into(),
        json!(with_whitespace(
            r"^((oci|https)://[^{WS}/?#@]+([/?#][^{WS}]*)?|git\+https://[^{WS}/?#@]+([/?#][^{WS}]*)?@[0-9a-f]{40})(?![\s\S])"
        )),
    );
}

fn lock_source(schema: &mut Schema) {
    // The first two alternatives are the URL forms: no white space, no user-info in the
    // authority. The third is a path: no leading slash, white space or colon, no backslash,
    // colon or line break anywhere, and no `..` segment.
    schema.insert(
        "pattern".into(),
        json!(with_whitespace(
            r"^((oci|https)://(?=[^{WS}@])[^{WS}/?#@]*([/?#][^{WS}]*)?|git\+https://(?=[^{WS}@])[^{WS}/?#@]*([/?#][^{WS}]*)?@[0-9a-f]{40}|(?=[^{WS}/\\:])(?!([^/]*/)*\.\.(/|(?![\s\S])))[^\\:\n\r\u2028\u2029]*)(?![\s\S])"
        )),
    );
}

/// What a plugin is, as the index and the lock list it.
#[derive(JsonSchema)]
#[schemars(rename_all = "lowercase", inline)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the variants and no code builds one"
)]
enum Kind {
    Data,
    Code,
}

/// The plugin index document.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct PackIndex {
    schema_version: SchemaVersion,
    /// One entry per plugin, sorted by namespace and then id, with no pair repeated. At most
    /// 100000 entries.
    #[schemars(extend("maxItems" = INDEX_ENTRIES))]
    plugins: Vec<PackIndexEntry>,
}

/// Semantic version 2.0.0 without build metadata. Major, minor and patch have at most nine
/// digits (at most 999999999), which is the bound the reader enforces. Otherwise the pattern is
/// the `Semver` definition of `common.schema.json`.
#[derive(JsonSchema)]
#[schemars(
    with = "String",
    extend("pattern" = r"^(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})\.(0|[1-9][0-9]{0,8})(-(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)(\.(0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*))*)?(?![\s\S])")
)]
struct PackIndexSemver;

/// Where to fetch the plugin: an oci:// reference, an https:// release archive URL, or a
/// git+https:// URL pinned to a full commit hash after @. The URL carries no user-info. http://
/// and path sources are invalid: a public listing has no project root.
#[derive(JsonSchema)]
#[schemars(
    with = "String",
    transform = index_source,
    extend("maxLength" = INDEX_SOURCE_CHARS)
)]
struct PackIndexSource;

/// The key the publisher signs with, for display. Never a trust anchor: a first install shows
/// the fingerprint and waits for the explicit pinning command, and a pinned key wins over this
/// value.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct PublisherKey {
    /// Minisign key id, 16 uppercase hex digits.
    #[schemars(extend("pattern" = r"^[0-9A-F]{16}(?![\s\S])"))]
    key_id: String,
    /// SHA-256 of the raw public key bytes, 64 lowercase hex digits. This is the value that
    /// `slopvac plugins pin` takes.
    #[schemars(extend("pattern" = r"^[0-9a-f]{64}(?![\s\S])"))]
    fingerprint: String,
}

/// One listed plugin.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct PackIndexEntry {
    /// Plugin id from the plugin manifest. `core` is reserved and never listed.
    #[schemars(extend("maxLength" = INDEX_ID_CHARS))]
    id: PluginId,
    /// Publisher namespace that the key and the source are bound to. Same character set as a
    /// plugin id. `core` is reserved and never listed. An entry may claim `slopvac`, but only the
    /// compiled plugins key verifies that namespace.
    #[schemars(extend("maxLength" = INDEX_ID_CHARS))]
    namespace: PluginId,
    /// Display name. Text from the index: a reader escapes control characters before it prints.
    #[schemars(length(min = 1, max = 128))]
    name: String,
    /// One-line summary. Text from the index: a reader escapes control characters before it
    /// prints.
    #[schemars(length(min = 1, max = 1024))]
    description: String,
    /// Latest listed version.
    version: PackIndexSemver,
    /// `data` for a data plugin, `code` for a code plugin. The same values as the manifest kind.
    kind: Kind,
    /// Lowest plugin extension API the host must implement. A host with a lower API cannot load
    /// the plugin. At most 4294967295, the bound the reader enforces. A whole number written with
    /// a fraction, such as 2.0, is an integer here and in the reader.
    #[schemars(extend("maximum" = INDEX_API))]
    slopvac_api: ApiVersion,
    source: PackIndexSource,
    publisher_key: PublisherKey,
}

/// The plugin lockfile document.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
pub(crate) struct PluginsLock {
    schema_version: SchemaVersion,
    /// Locked entries, sorted by id, so diffs stay small. At most 1000 entries.
    #[schemars(extend("maxItems" = LOCK_PACKS))]
    plugins: Vec<Locked>,
}

/// SHA-256 digest of the plugin content, as the OCI manifest digest or the hash of the
/// extracted tree.
#[derive(JsonSchema)]
#[schemars(
    with = "String",
    extend("pattern" = r"^sha256:[0-9a-f]{64}(?![\s\S])")
)]
struct Digest;

/// The minisign signature the host verified.
#[derive(JsonSchema)]
#[schemars(inline, transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct Signature {
    /// Minisign key id, 16 hex digits.
    #[schemars(extend("pattern" = r"^[0-9A-F]{16}(?![\s\S])"))]
    key_id: String,
    /// The notBefore time of the signed statement, in UTC (RFC 3339, `Z` suffix). The pattern
    /// rejects an impossible month, day, hour, minute or second, and 30 or 31 February. 29
    /// February of a non-leap year passes the schema and fails when the host parses the time.
    /// `plugins update` refuses an older value than the lockfile holds unless the user passes the
    /// downgrade flag.
    #[schemars(extend(
        "pattern" = r"^[0-9]{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12][0-9]|3[01])|(?:0[469]|11)-(?:0[1-9]|[12][0-9]|30)|02-(?:0[1-9]|1[0-9]|2[0-9]))T(?:[01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]Z(?![\s\S])",
        "format" = "date-time"
    ))]
    not_before: String,
}

/// One locked plugin.
#[derive(JsonSchema)]
#[schemars(transform = x_extensions)]
#[expect(
    dead_code,
    reason = "schema only: the derive reads the fields and no code reads a value"
)]
struct Locked {
    id: PluginId,
    /// `data` for a data plugin, `code` for a code plugin. The same values as the manifest kind.
    kind: Kind,
    version: Semver,
    slopvac_api: ApiVersion,
    /// An oci:// reference, an https:// release archive URL, a git+https:// URL pinned to a
    /// full commit hash after @, or a path relative to the project root. A URL carries no
    /// user-info. A path has no colon, no backslash, no `..` segment and no line break, and it
    /// starts with a character that is neither white space nor a slash. http:// is rejected. The
    /// reader also refuses control characters and other unsafe characters anywhere in the value,
    /// and a value over 4096 bytes. The schema counts characters, so it caps the value at 4096
    /// of them.
    #[schemars(with = "String", transform = lock_source, extend("maxLength" = LOCK_SOURCE_CAP))]
    source: String,
    digest: Digest,
    /// Present when the host verified a minisign signature.
    #[schemars(transform = absent_not_null)]
    signature: Option<Signature>,
    /// Ids of locked plugins this entry needs. At most 256.
    #[schemars(transform = absent_not_null, extend("uniqueItems" = true, "maxItems" = LOCK_REQUIRES))]
    requires: Option<Vec<PackId>>,
}
