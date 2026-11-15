//! The migration guide, kept in one source for `slopvac prime migrate` and
//! `docs/migrate-2-to-3.md`.

const GUIDE: &str = include_str!("../data/migrate-guide.md");

const DOC_NOTICE: &str = "<!-- Generated from crates/slopvac-migrate/data/migrate-guide.md. \
Edit that file and run `UPDATE_DOCS=1 cargo test -p slopvac-migrate --test guide`. -->\n\n";

/// The text `slopvac prime migrate` prints.
#[must_use]
pub fn prime_migrate() -> &'static str {
    GUIDE
}

/// The text of `docs/migrate-2-to-3.md`: the guide under a generated-file notice.
#[must_use]
pub fn doc() -> String {
    format!("{DOC_NOTICE}{GUIDE}")
}
