//! Work done by the section-scoped and text taggers grows linearly with the document.
//!
//! The tests count work instead of time: blocks examined to find section bounds, text
//! positions scanned, and bytes of heading and admonition text matched. They assert an upper
//! bound that grows linearly with the input, not the count of one traversal.
#![allow(clippy::unwrap_used, reason = "test code: a panic is a failed test")]

use rstest::rstest;

use slopvac_context::{
    BlockView, DocumentView, EnforcedInvariantTagger, PlainDocument, RequirementTagger, Tagger,
    TaggerSpec, take_admonition_steps, take_heading_steps, take_negation_steps, take_section_steps,
    take_text_steps,
};

fn doc(src: &str) -> PlainDocument {
    PlainDocument::from_markdown("a.md", src)
}

fn section_tagger() -> Box<dyn Tagger> {
    let yaml = "schema_version: 3\nid: t\ntag: k\nscope: section\nwhen:\n  heading_path: 'setup'\n";
    Box::new(TaggerSpec::from_yaml(yaml).unwrap().compile().unwrap())
}

/// `# Setup`, then `paragraphs` paragraphs: `1 + paragraphs` blocks in one section.
fn one_section(paragraphs: usize) -> String {
    format!("# Setup\n\n{}", "Paragraph.\n\n".repeat(paragraphs))
}

/// `sections` sections of `# Setup` and `paragraphs` paragraphs each.
fn many_sections(sections: usize, paragraphs: usize) -> String {
    one_section(paragraphs).repeat(sections)
}

#[rstest]
#[case::small(10)]
#[case::double(20)]
#[case::quadruple(40)]
fn a_matching_section_is_tagged_once_whatever_its_size(#[case] paragraphs: usize) {
    let doc = doc(&one_section(paragraphs));
    assert_eq!(section_tagger().tag(&doc).unwrap().len(), 1);
}

#[test]
fn every_matching_section_is_tagged_once() {
    let doc = doc(&many_sections(5, 3));
    assert_eq!(section_tagger().tag(&doc).unwrap().len(), 5);
}

#[test]
fn a_section_runs_through_its_subsections_to_the_end_of_the_document() {
    let src = "# Setup\n\nFirst.\n\n## Sub\n\nSecond.\n";
    let doc = doc(src);
    let tags = section_tagger().tag(&doc).unwrap();
    // The subsection's blocks also sit under the `Setup` heading, so both sections match.
    let texts: Vec<&str> = tags.iter().map(|t| &src[t.span.clone()]).collect();
    assert_eq!(
        texts,
        ["Setup\n\nFirst.\n\n## Sub\n\nSecond.", "Sub\n\nSecond."]
    );
}

/// Assert `steps(size) <= per_unit * size + slack` for sizes that double from about 1k to 8k.
///
/// A linear algorithm stays under a bound of this shape at every size. A quadratic one passes
/// at the smallest size, then overshoots the bound by a factor that doubles with each size,
/// so the largest size fails it by a wide margin.
fn assert_linear(
    what: &str,
    per_unit: usize,
    slack: usize,
    steps: impl Fn(usize) -> (usize, usize),
) {
    for size in [1_000, 2_000, 4_000, 8_000] {
        let (units, used) = steps(size);
        let bound = per_unit * units + slack;
        assert!(
            used <= bound,
            "{what}: {used} steps for {units} units at size {size}, bound {bound}"
        );
    }
}

/// Each block is examined once to find the section bounds, so the steps are at most the
/// number of blocks, and a few extra count as slack.
#[rstest]
#[case::one_section(1)]
#[case::sections_of_100_blocks(100)]
#[case::sections_of_10_blocks(10)]
fn finding_section_bounds_takes_linear_steps(#[case] paragraphs: usize) {
    assert_linear("section bounds", 1, 8, |blocks| {
        let sections = blocks / (1 + paragraphs);
        let doc = doc(&many_sections(sections.max(1), paragraphs));
        let _ = take_section_steps();
        section_tagger().tag(&doc).unwrap();
        (blocks, take_section_steps())
    });
}

/// Positions examined by `tagger` on one paragraph of `text`.
fn text_steps(tagger: &dyn Tagger, text: &str) -> usize {
    let doc = doc(&format!("# T\n\n{text}\n"));
    let _ = take_text_steps();
    tagger.tag(&doc).unwrap();
    take_text_steps()
}

fn text_taggers() -> [Box<dyn Tagger>; 2] {
    [
        Box::new(RequirementTagger::new().unwrap()),
        Box::new(EnforcedInvariantTagger::new().unwrap()),
    ]
}

/// `](` repeated has no `)` to close a link. Cleaning reads the text a few times at most: to
/// find backticks, to visit each character, and to look for the missing `)`. The bound is
/// four steps per byte.
#[test]
fn unclosed_link_openers_take_linear_steps() {
    for tagger in text_taggers() {
        assert_linear(tagger.id(), 4, 16, |bytes| {
            (bytes, text_steps(&*tagger, &"](".repeat(bytes / 2)))
        });
    }
}

/// Backtick runs of length `k` down to 1, each after an `a`, never find a closing run of
/// their length. The text has `k + k(k+1)/2` bytes. Cleaning reads the bytes to find the
/// runs, decides each run once, then visits each character: three steps per byte cover it.
#[test]
fn unclosed_backtick_runs_take_linear_steps() {
    for tagger in text_taggers() {
        for k in [45, 63, 90, 127] {
            let mut text = String::new();
            for l in (1..=k).rev() {
                text.push('a');
                text.push_str(&"`".repeat(l));
            }
            let bytes = text.len();
            let used = text_steps(&*tagger, &text);
            assert!(
                used <= 3 * bytes + 16,
                "{}: {used} steps for {bytes} bytes",
                tagger.id()
            );
        }
    }
}

/// A heading is matched against the heading pattern once, however many blocks sit under it.
/// The bound is two bytes per heading byte, so one pass over the heading plus slack passes.
#[test]
fn a_long_heading_is_matched_once_for_all_its_blocks() {
    assert_linear("heading match", 2, 64, |blocks| {
        let heading = "setup ".repeat(blocks / 8);
        let src = format!("# {heading}\n\n{}", "Paragraph.\n\n".repeat(blocks));
        let doc = doc(&src);
        let _ = take_heading_steps();
        section_tagger().tag(&doc).unwrap();
        (heading.len(), take_heading_steps())
    });
}

/// A sentence that opens with a negator and lists enforcement verbs without whitespace between
/// them is one long token before each later verb. Deciding whether a verb is negated looks at
/// the two tokens before it, so a scan that restarts from the token's start for every verb
/// reads the token again each time. The text has 8 bytes per verb; the bound is 4 steps per
/// byte, which one pass with the trim of each verb's trailing separators stays under.
#[rstest]
#[case::tight_commas(",")]
#[case::spaced_commas(", ")]
fn negated_enforcement_verbs_take_linear_steps(#[case] sep: &str) {
    let tagger = EnforcedInvariantTagger::new().unwrap();
    assert_linear("negation scan", 4, 64, |bytes| {
        let verbs = bytes / 8;
        let text = format!("Never {}.", vec!["rejects"; verbs].join(sep));
        let doc = doc(&format!("# T\n\n{text}\n"));
        let _ = take_negation_steps();
        tagger.tag(&doc).unwrap();
        (text.len(), take_negation_steps())
    });
}

/// Which blocks sit under a matching heading follows the heading path as it changes. A
/// heading block is under itself, so the matching heading is tagged too.
#[rstest]
#[case::sibling_heading_ends_the_match("# Setup\n\nA.\n\n# Other\n\nB.\n", &["Setup", "A."])]
#[case::subheading_keeps_the_match("# Setup\n\nA.\n\n## Sub\n\nB.\n", &["Setup", "A.", "Sub", "B."])]
#[case::match_only_on_the_outer_heading(
    "# Other\n\n## Setup\n\nA.\n\n## Next\n\nB.\n\n# Last\n\nC.\n",
    &["Setup", "A."]
)]
fn blocks_match_while_a_matching_heading_is_above_them(#[case] src: &str, #[case] want: &[&str]) {
    let yaml =
        "schema_version: 3\nid: t\ntag: k\nscope: paragraph\nwhen:\n  heading_path: 'setup'\n";
    let tagger = TaggerSpec::from_yaml(yaml).unwrap().compile().unwrap();
    let doc = PlainDocument::from_markdown("a.md", src);
    let got: Vec<&str> = tagger
        .tag(&doc)
        .unwrap()
        .iter()
        .map(|t| src[t.span.clone()].trim())
        .collect();
    assert_eq!(got, want);
}

/// A document whose paragraphs all sit under one admonition, as the engine reports them: every
/// block borrows the same title.
struct Admonished {
    inner: PlainDocument,
    title: String,
}

impl DocumentView for Admonished {
    fn path(&self) -> &str {
        self.inner.path()
    }
    fn title(&self) -> Option<&str> {
        self.inner.title()
    }
    fn front_matter(&self, key: &str) -> Option<&str> {
        self.inner.front_matter(key)
    }
    fn source_len(&self) -> usize {
        self.inner.source_len()
    }
    fn blocks(&self) -> Vec<BlockView<'_>> {
        let mut blocks = self.inner.blocks();
        for b in &mut blocks {
            b.admonition_title = Some(&self.title);
        }
        blocks
    }
}

/// A title is matched against the admonition pattern once, however many blocks sit under it.
#[test]
fn a_long_admonition_title_is_matched_once_for_all_its_blocks() {
    let yaml =
        "schema_version: 3\nid: t\ntag: k\nscope: paragraph\nwhen:\n  admonition_title: 'danger'\n";
    let tagger = TaggerSpec::from_yaml(yaml).unwrap().compile().unwrap();
    assert_linear("admonition match", 2, 64, |blocks| {
        let doc = Admonished {
            inner: PlainDocument::from_markdown("a.md", &"Paragraph.\n\n".repeat(blocks)),
            title: "calm ".repeat(blocks / 8),
        };
        let _ = take_admonition_steps();
        assert_eq!(tagger.tag(&doc).unwrap().len(), 0);
        (doc.title.len(), take_admonition_steps())
    });
}
