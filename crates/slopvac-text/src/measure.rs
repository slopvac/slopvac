//! The counts behind the metric rules. A port of the Python reference
//! (`engine.count_clause_boundaries`, `analyze.coordinated_items`, `analyze.longest_noun_stack`
//! and `metrics`). Every function reads text only; deciding which text to read is the
//! caller's job.

use std::sync::LazyLock;

use pcre2::bytes::Regex;

use crate::pattern::{TextError, find_all, matches_start, re, re_i};
use crate::pyrules::py_isspace;

/// A clause boundary is where a reader starts holding a second idea. Excluded on purpose:
/// a comma before a restrictive clause, a comma in a list of nouns, and a subordinating
/// conjunction that opens the sentence.
static CLAUSE_JOIN: LazyLock<Regex> = LazyLock::new(|| {
    re_i(concat!(
        r";|\s+(?:--|—|–)\s+",
        r"|,\s*(?:and|but|or|so|yet|then|while|whereas|although)\s+(?=\w+\s+\w)",
        r"|,\s*which\b",
        r"|,\s*(?:however|therefore|meanwhile|nevertheless|consequently)\b"
    ))
});

static COORDINATING_CONJUNCTION: LazyLock<Regex> =
    LazyLock::new(|| re_i(r",?\s+\b(?:and|or)\b\s+"));

static ADJECTIVE_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| re_i(r"\b\w{4,}(?:able|ible|ical|ful|less|ous|ive|istic|ary|ish|like)\b"));
static NOUN_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    re_i(r"\b\w{4,}(?:tion|sion|ment|ness|ity|ance|ence|ism|ology|er|or|ist|ure|age|ing)\b")
});

/// `**x**` and `__x__`, non-greedy and on one line.
static BOLD_SPAN: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?<!\*)\*\*(?!\s)[^*\n]+?(?<!\s)\*\*(?!\*)|__(?!\s)[^_\n]+?(?<!\s)__"));

/// The em dash and the double hyphen that stands in for it. The en dash is not an aside.
static DASH_AS_ASIDE: LazyLock<Regex> = LazyLock::new(|| re(r"—|(?<![-\w])--(?![-\w>])"));

/// A code span closes with a backtick run exactly as long as its opener.
static INLINE_CODE: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?s)(?<!`)(?P<ticks>`+)(?P<body>.*?)(?<!`)(?P=ticks)(?!`)"));

static HTML_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<!--.*?-->"));

/// A bullet whose visible content opens with a bold run and a colon, with the colon inside
/// or outside the emphasis.
static BOLD_COLON_BULLET: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"^\s*(?:[-*+]|\d+[.)])\s+(?:\*\*|__)[^*_]+?",
        r"(?::(?:\*\*|__)|(?:\*\*|__)\s*:)"
    ))
});

static TOKEN: LazyLock<Regex> = LazyLock::new(|| re(r"[\w'\x00-]+"));

/// How many times the sentence starts a new independent idea.
pub fn clause_boundaries(sentence: &str) -> Result<usize, TextError> {
    Ok(find_all(&CLAUSE_JOIN, sentence)?.len())
}

/// Items in the coordinated series of a sentence, or 0 without a conjunction.
///
/// Counts the commas before the first conjunction and adds two.
pub fn coordinated_items(sentence: &str) -> Result<usize, TextError> {
    let Some(&(start, _)) = find_all(&COORDINATING_CONJUNCTION, sentence)?.first() else {
        return Ok(0);
    };
    Ok(sentence[..start].matches(',').count() + 2)
}

/// Words that never carry a noun stack even though they sit between nouns: articles,
/// prepositions, conjunctions, subordinators, determiners, pronouns, modals, quantifiers and
/// common verbs that a suffix test cannot tell from nouns. Sorted, for `binary_search`.
const STACK_BREAKER: [&str; 247] = [
    "a",
    "about",
    "across",
    "add",
    "additional",
    "adds",
    "after",
    "against",
    "all",
    "along",
    "although",
    "among",
    "an",
    "and",
    "another",
    "any",
    "appear",
    "appears",
    "applies",
    "apply",
    "are",
    "around",
    "as",
    "ask",
    "asked",
    "at",
    "be",
    "because",
    "been",
    "before",
    "begin",
    "begins",
    "behind",
    "being",
    "between",
    "beyond",
    "both",
    "but",
    "by",
    "can",
    "cannot",
    "carries",
    "carry",
    "certain",
    "clear",
    "clears",
    "could",
    "count",
    "counts",
    "despite",
    "different",
    "do",
    "does",
    "drop",
    "drops",
    "during",
    "each",
    "either",
    "end",
    "ends",
    "every",
    "exit",
    "exits",
    "fail",
    "fails",
    "few",
    "fire",
    "fires",
    "five",
    "folding",
    "for",
    "four",
    "from",
    "get",
    "gets",
    "give",
    "gives",
    "had",
    "has",
    "have",
    "he",
    "her",
    "here",
    "his",
    "hold",
    "holds",
    "how",
    "if",
    "important",
    "in",
    "including",
    "inside",
    "into",
    "is",
    "it",
    "its",
    "keep",
    "keeps",
    "land",
    "lands",
    "less",
    "lint",
    "linting",
    "load",
    "loads",
    "lose",
    "loses",
    "make",
    "makes",
    "many",
    "may",
    "mean",
    "means",
    "might",
    "more",
    "most",
    "multiple",
    "must",
    "my",
    "name",
    "names",
    "need",
    "needs",
    "neither",
    "no",
    "nor",
    "not",
    "numerous",
    "of",
    "on",
    "once",
    "one",
    "onto",
    "open",
    "opens",
    "or",
    "other",
    "our",
    "outside",
    "over",
    "overlapping",
    "own",
    "owns",
    "particular",
    "pass",
    "passes",
    "per",
    "pick",
    "picks",
    "put",
    "puts",
    "reach",
    "reaches",
    "read",
    "reads",
    "report",
    "reports",
    "resolve",
    "resolves",
    "return",
    "returns",
    "run",
    "runs",
    "same",
    "say",
    "says",
    "set",
    "sets",
    "several",
    "shall",
    "she",
    "should",
    "show",
    "shows",
    "significant",
    "since",
    "sit",
    "sits",
    "so",
    "some",
    "specific",
    "start",
    "starts",
    "stay",
    "stays",
    "subtract",
    "subtracts",
    "such",
    "switch",
    "switches",
    "take",
    "takes",
    "than",
    "that",
    "the",
    "their",
    "them",
    "then",
    "there",
    "these",
    "they",
    "this",
    "those",
    "though",
    "three",
    "through",
    "throughout",
    "to",
    "toward",
    "towards",
    "track",
    "tracks",
    "two",
    "under",
    "unless",
    "until",
    "upon",
    "us",
    "use",
    "uses",
    "using",
    "various",
    "versus",
    "via",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "whereas",
    "whether",
    "which",
    "while",
    "who",
    "whom",
    "whose",
    "why",
    "will",
    "win",
    "wins",
    "with",
    "within",
    "without",
    "would",
    "you",
    "your",
];

/// The `-ed` words that are ordinary nouns.
const PARTICIPLE_NOUNS: [&str; 13] = [
    "speed", "breed", "creed", "steed", "tweed", "thread", "spread", "bread", "ahead", "embed",
    "shred", "sacred", "hundred",
];

/// The `-able` words that are ordinary nouns and can head a real stack.
const ADJECTIVE_NOUNS: [&str; 12] = [
    "variable",
    "executable",
    "deliverable",
    "observable",
    "callable",
    "iterable",
    "awaitable",
    "consumable",
    "disposable",
    "renewable",
    "receivable",
    "payable",
];

/// Singular words ending in a bare `s`.
const S_ENDING_SINGULARS: [&str; 9] = [
    "news", "series", "species", "alias", "canvas", "atlas", "bias", "lens", "chaos",
];

const NOUN_SUFFIXES: [&str; 15] = [
    "tion", "sion", "ment", "ness", "ity", "ance", "ence", "ism", "ology", "er", "or", "ist",
    "ure", "age", "ing",
];

/// Whether `word` (ASCII letters) is a noun by `NOUN_SUFFIX`: four letters, then a suffix.
fn has_noun_suffix(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    NOUN_SUFFIXES
        .iter()
        .any(|s| lower.ends_with(s) && lower.len() >= 4 + s.len())
}

/// A past participle cannot head a stack: five letters and `ed`, and not an ordinary noun.
fn is_participle(word: &str) -> bool {
    word.len() >= 7
        && word.ends_with("ed")
        && !PARTICIPLE_NOUNS.contains(&word.to_ascii_lowercase().as_str())
}

/// An `-able` or `-ible` adjective cannot head a stack either.
fn is_predicate_adjective(word: &str) -> bool {
    word.len() >= 7
        && (word.ends_with("able") || word.ends_with("ible"))
        && !ADJECTIVE_NOUNS.contains(&word.to_ascii_lowercase().as_str())
}

/// Whether `word` must be the last word of any stack it sits in: a lowercase plural or
/// third-person verb.
fn ends_stack(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    word.starts_with(|c: char| c.is_ascii_lowercase())
        && lower.len() >= 4
        && lower.ends_with('s')
        && !["ss", "us", "is", "ics"].iter().any(|s| lower.ends_with(s))
        && !S_ENDING_SINGULARS.contains(&lower.as_str())
}

/// Words in a stack, counting a run of capitalised words as one name. A stack of only
/// capitalised words counts every word.
fn stack_units(words: &[&str]) -> usize {
    let capitalised = |w: &&str| w.starts_with(|c: char| c.is_ascii_uppercase());
    if words.iter().all(capitalised) {
        return words.len();
    }
    let mut units = 0;
    let mut previous = false;
    for word in words {
        let current = capitalised(word);
        if !(current && previous) {
            units += 1;
        }
        previous = current;
    }
    units
}

/// The units of a run once a trailing participle or predicate adjective is cut, when a
/// word of the run carries a noun suffix.
fn closed_stack(run: &[&str]) -> usize {
    let mut end = run.len();
    while end > 0 && (is_participle(run[end - 1]) || is_predicate_adjective(run[end - 1])) {
        end -= 1;
    }
    let trimmed = &run[..end];
    if trimmed.is_empty() || !trimmed.iter().any(|w| has_noun_suffix(w)) {
        return 0;
    }
    stack_units(trimmed)
}

/// Words in the longest run of consecutive stackable words in a sentence.
///
/// A run ends at an article, a preposition, an `-ly` adverb, punctuation, a token that is
/// not plain letters, and after a plural or verb. It counts only when a word of it carries
/// a noun suffix. This under-reports on purpose: it is a signal, not a tagger.
pub fn longest_noun_stack(sentence: &str) -> Result<usize, TextError> {
    let mut longest = 0;
    let mut run: Vec<&str> = Vec::new();
    let mut previous_end = 0;
    for (start, end) in find_all(&TOKEN, sentence)? {
        let gap = &sentence[previous_end..start];
        previous_end = end;
        let token = sentence[start..end].trim_matches('\'');
        let broken = !gap.is_empty() && !gap.chars().all(py_isspace);
        let lower = token.to_ascii_lowercase();
        let stackable = !token.is_empty()
            && token.bytes().all(|b| b.is_ascii_alphabetic())
            && STACK_BREAKER.binary_search(&lower.as_str()).is_err()
            && !lower.ends_with("ly");
        if broken || !stackable {
            longest = longest.max(closed_stack(&run));
            run.clear();
        }
        if !stackable {
            continue;
        }
        run.push(token);
        if ends_stack(token) {
            longest = longest.max(closed_stack(&run));
            run.clear();
        }
    }
    Ok(longest.max(closed_stack(&run)))
}

/// Words with an adjective suffix and words with a noun suffix in `text`, in that order.
pub fn suffix_counts(text: &str) -> Result<(usize, usize), TextError> {
    Ok((
        find_all(&ADJECTIVE_SUFFIX, text)?.len(),
        find_all(&NOUN_SUFFIX, text)?.len(),
    ))
}

/// Runs of ASCII letters and apostrophes, the words that the density metrics divide by.
#[must_use]
pub fn plain_words(text: &str) -> usize {
    let mut count = 0;
    let mut inside = false;
    for b in text.bytes() {
        let word = b.is_ascii_alphabetic() || b == b'\'';
        if word && !inside {
            count += 1;
        }
        inside = word;
    }
    count
}

/// Bold spans in markup text.
pub fn bold_spans(markup: &str) -> Result<usize, TextError> {
    Ok(find_all(&BOLD_SPAN, markup)?.len())
}

/// Em dashes and double hyphens used as an aside in markup text.
pub fn dashes_as_aside(markup: &str) -> Result<usize, TextError> {
    Ok(find_all(&DASH_AS_ASIDE, markup)?.len())
}

/// Markup text with inline code replaced by a space and HTML comments blanked. A comment
/// keeps its line breaks so later lines stay where they were.
pub fn without_code_and_comments(markup: &str) -> Result<String, TextError> {
    let mut out = String::with_capacity(markup.len());
    let mut last = 0;
    for (start, end) in find_all(&INLINE_CODE, markup)? {
        out.push_str(&markup[last..start]);
        out.push(' ');
        last = end;
    }
    out.push_str(&markup[last..]);
    let mut blanked = String::with_capacity(out.len());
    last = 0;
    for (start, end) in find_all(&HTML_COMMENT, &out)? {
        blanked.push_str(&out[last..start]);
        blanked.extend(
            out[start..end]
                .chars()
                .map(|c| if c == '\n' { c } else { ' ' }),
        );
        last = end;
    }
    blanked.push_str(&out[last..]);
    Ok(blanked)
}

/// Whether a raw line is a bullet that opens with a bold run and a colon.
pub fn is_bold_colon_bullet(line: &str) -> Result<bool, TextError> {
    matches_start(&BOLD_COLON_BULLET, line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stack_breaker_list_is_sorted_for_binary_search() {
        assert!(STACK_BREAKER.is_sorted());
    }

    #[test]
    fn a_preposition_a_modal_and_a_common_verb_end_a_stack() {
        // `in`, `must` and `reads` are in the list, so no run reaches 2 words.
        let n = longest_noun_stack("storage in migration must configuration reads storage");
        assert_eq!(n.unwrap(), 1);
    }

    #[test]
    fn a_semicolon_and_a_comma_conjunction_each_start_a_clause() {
        assert_eq!(
            clause_boundaries("We read it; we parse it, and then we emit it.").unwrap(),
            2
        );
    }

    #[test]
    fn a_comma_before_a_conjunction_with_one_word_after_is_not_a_boundary() {
        // The lookahead wants two words after the conjunction.
        assert_eq!(clause_boundaries("Read it, and stop.").unwrap(), 0);
    }

    #[test]
    fn a_sentence_with_no_conjunction_has_no_series() {
        assert_eq!(coordinated_items("Red, green, blue.").unwrap(), 0);
    }

    #[test]
    fn a_series_counts_its_commas_before_the_first_conjunction_plus_two() {
        assert_eq!(coordinated_items("We ship a, b, c, and d.").unwrap(), 4);
        assert_eq!(coordinated_items("We ship a and b, c, d.").unwrap(), 2);
    }

    #[test]
    fn a_stack_with_a_noun_suffix_counts_its_words() {
        let n = longest_noun_stack("The container orchestration platform migration strategy.");
        assert_eq!(n.unwrap(), 5);
    }

    #[test]
    fn a_stack_without_a_noun_suffix_is_not_counted() {
        assert_eq!(longest_noun_stack("disk cache size limit").unwrap(), 0);
    }

    #[test]
    fn an_article_ends_a_stack() {
        let n = longest_noun_stack("migration strategy of the container orchestration platform");
        assert_eq!(n.unwrap(), 3);
    }

    #[test]
    fn a_comma_ends_a_stack() {
        // Each word has a noun suffix; without the comma the stack would be 2.
        assert_eq!(longest_noun_stack("storage, migration").unwrap(), 1);
    }

    #[test]
    fn a_trailing_participle_is_cut_from_the_stack() {
        let n = longest_noun_stack("distributed cache invalidation strategy");
        assert_eq!(n.unwrap(), 4);
        let n = longest_noun_stack("cache invalidation strategy distributed");
        assert_eq!(n.unwrap(), 3);
    }

    #[test]
    fn a_plural_ends_the_stack() {
        // `accounts` ends the first run, which has no noun suffix. `migration plan` is 2.
        let n = longest_noun_stack("user accounts migration plan");
        assert_eq!(n.unwrap(), 2);
    }

    #[test]
    fn a_run_of_capitalised_words_inside_a_mixed_stack_is_one_name() {
        // Five words, but `New Zealand` is one unit.
        let n = longest_noun_stack("New Zealand provincial migration unit");
        assert_eq!(n.unwrap(), 4);
    }

    #[test]
    fn suffix_counts_separate_adjectives_from_nouns() {
        // beautiful, careless: adjectives. migration, strategy: only migration has a suffix.
        assert_eq!(
            suffix_counts("beautiful careless migration strategy").unwrap(),
            (2, 1)
        );
    }

    #[test]
    fn plain_words_count_letter_and_apostrophe_runs() {
        assert_eq!(plain_words("don't use x-ray 42 times"), 5);
    }

    #[test]
    fn bold_spans_count_both_marker_forms() {
        assert_eq!(bold_spans("**a** and __b__ but ** c** or **d").unwrap(), 2);
    }

    #[test]
    fn dashes_count_the_em_dash_and_the_double_hyphen_aside() {
        let text = "a — b -- c a--b --flag a–b";
        assert_eq!(dashes_as_aside(text).unwrap(), 2);
    }

    #[test]
    fn inline_code_and_comments_are_removed_from_markup() {
        let out = without_code_and_comments("a `x` b <!-- c\nd --> e").unwrap();
        assert_eq!(out, "a   b       \n      e");
    }

    #[test]
    fn a_bold_colon_bullet_allows_the_colon_inside_or_outside() {
        for (line, want) in [
            ("- **Thing:** text", true),
            ("1. __Thing__: text", true),
            ("  * **Thing** : text", true),
            ("- **just bold** text", false),
            ("**Thing:** text", false),
        ] {
            assert_eq!(is_bold_colon_bullet(line).unwrap(), want, "{line}");
        }
    }
}
