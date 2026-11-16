//! Prints `file<TAB>line<TAB>char column<TAB>word<TAB>tag` for every token of
//! the given files. Lines are 1-based and columns count characters from 1, the
//! way Vale reports positions. Used by `tools/full_corpus_agreement.py`.

use std::process::ExitCode;

use slopvac_pos::Tagger;

#[expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "this example is a command-line tool that reports on the standard streams"
)]
fn main() -> ExitCode {
    let tagger = Tagger::english();
    let mut status = ExitCode::SUCCESS;
    for path in std::env::args().skip(1) {
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("{path}: {err}");
                status = ExitCode::FAILURE;
                continue;
            }
        };
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        // `PATH.blocks` lists one block per line as "start end" byte offsets;
        // without it the whole file is one block.
        let blocks: Vec<std::ops::Range<usize>> = std::fs::read_to_string(format!("{path}.blocks"))
            .map_or_else(
                |_| std::iter::once(0..text.len()).collect(),
                |list| {
                    list.lines()
                        .filter_map(|l| {
                            let (a, b) = l.split_once(' ')?;
                            Some(a.parse().ok()?..b.parse().ok()?)
                        })
                        .collect()
                },
            );
        for token in tagger.tag_blocks(&text, &blocks) {
            let line = line_starts.partition_point(|&s| s <= token.span.start);
            let start = line_starts[line - 1];
            let column = text[start..token.span.start].chars().count() + 1;
            let text = token.text.replace(['\t', '\n', '\r'], " ");
            println!("{path}\t{line}\t{column}\t{text}\t{}", token.tag);
        }
    }
    status
}
