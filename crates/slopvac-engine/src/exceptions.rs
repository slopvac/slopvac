//! Exception detectors: the code that drops a finding for a named reason.
//!
//! A rule lists its exceptions by name. An exception with a `detector` drops a finding on
//! its own, with no annotation in the document. [`Detectors`] resolves the detector name:
//!
//! - `quotation` and `code-span` are built in.
//! - A name registered with [`Detectors::register`] runs that detector.
//! - Any other name is a context tag, so `requirement` and `enforced-invariant` come from
//!   the taggers in `slopvac-context`, and a pack's own tag works the same way. The finding
//!   is dropped when that tag covers its span.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::LazyLock;

use slopvac_model::Span;
use slopvac_parse::Document;
use slopvac_text::Pattern;

/// A candidate finding, as a detector sees it.
#[derive(Debug, Clone, Copy)]
pub struct MatchSite<'a> {
    /// The text the rule matched in: a block, a sentence or a line of projected prose.
    pub text: &'a str,
    /// The match, as byte offsets into `text`.
    pub range: &'a Range<usize>,
    /// The match in the source.
    pub span: Span,
    /// The parsed document.
    pub document: &'a Document,
    /// Context tags that cover `span`.
    pub tags: &'a [&'a str],
}

/// A detector for one exception name.
pub trait Detector: Send + Sync {
    /// The exception `detector` name this answers to.
    fn name(&self) -> &str;
    /// Whether the exception holds at `site`, so the finding is dropped.
    fn detects(&self, site: &MatchSite<'_>) -> bool;
}

/// A detector name was registered twice.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("detector `{0}` is already registered")]
pub struct DuplicateDetector(String);

/// A match wholly inside a quotation on its line. A document that bans a phrase has to
/// print the phrase. The scan is line-scoped and capped at 200 characters, because a quote
/// mark is also an apostrophe, a unit of inches and a shell quote. An unbounded scan would
/// pair marks that were never a quotation and silence half a document. There is no
/// straight single-quote branch for the same reason.
struct Quotation;

// Port of `engine._QUOTED_SPAN`. The `\w` classes are rewritten by `slopvac-text`.
static QUOTED_SPAN: LazyLock<Pattern> = LazyLock::new(|| {
    Pattern::new(
        r#"(?<![\w"”])"[^"\n]{1,200}(?<!\s)"(?!\w)|(?<![\w“])“[^”\n]{1,200}”|(?<![\w‘])‘[^’\n]{1,200}’"#,
        false,
    )
    .expect("the quotation pattern compiles")
});

impl Detector for Quotation {
    fn name(&self) -> &'static str {
        "quotation"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        // A pattern failure leaves the finding in place: the exception only ever removes.
        QUOTED_SPAN.find_all(site.text).is_ok_and(|quotes| {
            quotes.iter().any(|q| {
                // One byte for each quote mark would be wrong for the curly marks, so the
                // bound is the mark's own width, found from the text.
                let open = site.text[q.start..]
                    .chars()
                    .next()
                    .map_or(1, char::len_utf8);
                let close = site.text[..q.end]
                    .chars()
                    .next_back()
                    .map_or(1, char::len_utf8);
                q.start + open <= site.range.start && site.range.end + close <= q.end
            })
        })
    }
}

/// A match inside an inline code span.
struct CodeSpan;

impl Detector for CodeSpan {
    fn name(&self) -> &'static str {
        "code-span"
    }

    fn detects(&self, site: &MatchSite<'_>) -> bool {
        site.document
            .code_spans
            .iter()
            .any(|c| c.start() <= site.span.start() && site.span.end() <= c.end())
    }
}

/// The detectors of a run.
pub struct Detectors {
    by_name: BTreeMap<String, Box<dyn Detector>>,
}

impl Default for Detectors {
    fn default() -> Self {
        Self::builtin()
    }
}

impl Detectors {
    /// The built-in detectors: `quotation` and `code-span`.
    #[must_use]
    pub fn builtin() -> Self {
        let mut detectors = Self {
            by_name: BTreeMap::new(),
        };
        for detector in [Box::new(Quotation) as Box<dyn Detector>, Box::new(CodeSpan)] {
            detectors
                .register(detector)
                .expect("built-in detector names are distinct");
        }
        detectors
    }

    /// Add a detector.
    pub fn register(&mut self, detector: Box<dyn Detector>) -> Result<(), DuplicateDetector> {
        let name = detector.name().to_owned();
        if self.by_name.contains_key(&name) {
            return Err(DuplicateDetector(name));
        }
        self.by_name.insert(name, detector);
        Ok(())
    }

    /// Registered names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str)
    }

    /// Whether the exception `detector` holds at `site`. A name that is not registered is
    /// a context tag.
    #[must_use]
    pub fn detects(&self, detector: &str, site: &MatchSite<'_>) -> bool {
        self.by_name
            .get(detector)
            .map_or_else(|| site.tags.contains(&detector), |d| d.detects(site))
    }
}
