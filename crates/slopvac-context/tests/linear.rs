//! Work done by the section-scoped and text taggers grows linearly with the document.
//!
//! The tests count work instead of time: calls on the document view, blocks examined to
//! find section bounds, and tags emitted.

use std::cell::Cell;

use rstest::rstest;

use slopvac_context::{
    BlockView, DocumentView, EnforcedInvariantTagger, PlainDocument, RequirementTagger,
    SentenceView, Tagger, TaggerSpec, work,
};

/// A document that counts the calls a tagger makes on it.
struct Counting {
    inner: PlainDocument,
    blocks_calls: Cell<usize>,
    sentences_calls: Cell<usize>,
}

impl Counting {
    fn new(src: &str) -> Self {
        Self {
            inner: PlainDocument::from_markdown("a.md", src),
            blocks_calls: Cell::new(0),
            sentences_calls: Cell::new(0),
        }
    }
}

impl DocumentView for Counting {
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
        self.blocks_calls.set(self.blocks_calls.get() + 1);
        self.inner.blocks()
    }
    fn sentences<'a>(&self, block: &BlockView<'a>) -> Vec<SentenceView<'a>> {
        self.sentences_calls.set(self.sentences_calls.get() + 1);
        self.inner.sentences(block)
    }
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
    let doc = Counting::new(&one_section(paragraphs));
    assert_eq!(section_tagger().tag(&doc).unwrap().len(), 1);
}

#[test]
fn every_matching_section_is_tagged_once() {
    let doc = Counting::new(&many_sections(5, 3));
    assert_eq!(section_tagger().tag(&doc).unwrap().len(), 5);
}

#[test]
fn a_section_runs_through_its_subsections_to_the_end_of_the_document() {
    let src = "# Setup\n\nFirst.\n\n## Sub\n\nSecond.\n";
    let doc = Counting::new(src);
    let tags = section_tagger().tag(&doc).unwrap();
    // The subsection's blocks also sit under the `Setup` heading, so both sections match.
    let texts: Vec<&str> = tags.iter().map(|t| &src[t.span.clone()]).collect();
    assert_eq!(
        texts,
        ["Setup\n\nFirst.\n\n## Sub\n\nSecond.", "Sub\n\nSecond."]
    );
}

/// Blocks are `sections * (1 + paragraphs)`; each is examined once.
#[rstest]
#[case::one_section_of_11(1, 10, 11)]
#[case::one_section_of_21(1, 20, 21)]
#[case::one_section_of_41(1, 40, 41)]
#[case::five_sections(5, 3, 20)]
#[case::ten_sections(10, 3, 40)]
fn finding_section_bounds_examines_each_block_once(
    #[case] sections: usize,
    #[case] paragraphs: usize,
    #[case] blocks: usize,
) {
    let doc = Counting::new(&many_sections(sections, paragraphs));
    let _ = work::take_section_steps();
    section_tagger().tag(&doc).unwrap();
    assert_eq!(work::take_section_steps(), blocks);
    assert_eq!(doc.blocks_calls.get(), 1);
    assert_eq!(doc.sentences_calls.get(), 0);
}

/// Positions examined by `tagger` on one paragraph of `text`.
fn text_steps(tagger: &dyn Tagger, text: &str) -> usize {
    let doc = Counting::new(&format!("# T\n\n{text}\n"));
    let _ = work::take_text_steps();
    tagger.tag(&doc).unwrap();
    work::take_text_steps()
}

fn text_taggers() -> [Box<dyn Tagger>; 2] {
    [
        Box::new(RequirementTagger::new().unwrap()),
        Box::new(EnforcedInvariantTagger::new().unwrap()),
    ]
}

/// `](` repeated `n` times is `2n` characters and has no `)` to close a link. Cleaning scans
/// the text for backticks (`2n`), visits each character (`2n`), and looks for the missing `)`
/// once (`2n - 2` characters after the first `](`).
#[rstest]
#[case::fifty(50, 298)]
#[case::hundred(100, 598)]
#[case::two_hundred(200, 1198)]
fn unclosed_link_openers_are_scanned_for_a_closer_once(#[case] n: usize, #[case] steps: usize) {
    for tagger in text_taggers() {
        assert_eq!(
            text_steps(&*tagger, &"](".repeat(n)),
            steps,
            "{}",
            tagger.id()
        );
    }
}

/// Backtick runs of length `k` down to 1, each after an `a`, never find a closing run of
/// their length. The text has `n = k + k(k+1)/2` bytes and `k` runs. Cleaning reads the bytes
/// once to find the runs (`n`), decides each run once (`k`), then visits each character (`n`).
#[rstest]
#[case::ten(10, 140)]
#[case::twenty(20, 480)]
#[case::forty(40, 1760)]
fn unclosed_backtick_runs_are_matched_in_one_pass(#[case] k: usize, #[case] steps: usize) {
    let mut text = String::new();
    for l in (1..=k).rev() {
        text.push('a');
        text.push_str(&"`".repeat(l));
    }
    for tagger in text_taggers() {
        assert_eq!(text_steps(&*tagger, &text), steps, "{}", tagger.id());
    }
}
