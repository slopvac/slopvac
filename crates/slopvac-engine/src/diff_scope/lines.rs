//! Translate git's line numbers into the numbers findings carry.
//!
//! Git counts a line per LF. Findings come from `SourceIndex`, where LF, CRLF and a CR
//! that does not precede LF each end a line. The two agree unless the file holds a lone
//! CR, which splits one git line into several source lines.

use std::ops::RangeInclusive;

/// The first and last source line of each git line, in git order.
fn git_lines(text: &str) -> Vec<(u32, u32)> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut source = 1u32;
    let mut first = 1u32;
    for (i, byte) in bytes.iter().enumerate() {
        match byte {
            b'\n' => {
                lines.push((first, source));
                source = source.saturating_add(1);
                first = source;
            }
            // A CR ends a source line unless an LF follows; the LF then counts.
            b'\r' if bytes.get(i + 1) != Some(&b'\n') => source = source.saturating_add(1),
            _ => {}
        }
    }
    lines.push((first, source));
    lines
}

/// Whether some CR is not followed by LF.
fn has_lone_cr(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes
        .iter()
        .enumerate()
        .any(|(i, b)| *b == b'\r' && bytes.get(i + 1) != Some(&b'\n'))
}

/// Map ranges in git line numbers to source line numbers for `text`.
///
/// A whole-file range stays whole. A range that starts past the last line is dropped.
pub(super) fn to_source_lines(
    text: &str,
    ranges: &[RangeInclusive<u32>],
) -> Vec<RangeInclusive<u32>> {
    if !has_lone_cr(text) {
        return ranges.to_vec();
    }
    let lines = git_lines(text);
    let mut mapped: Vec<RangeInclusive<u32>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if *range.end() == u32::MAX {
            mapped.push(range.clone());
            continue;
        }
        let at = |n: usize| n.checked_sub(1).and_then(|i| lines.get(i));
        let last = (*range.end() as usize).min(lines.len());
        let (Some(start), Some(end)) = (at(*range.start() as usize), at(last)) else {
            continue;
        };
        let (from, to) = (start.0, end.1);
        match mapped.last_mut() {
            Some(last) if from <= last.end().saturating_add(1) => {
                *last = *last.start()..=(*last.end()).max(to);
            }
            _ => mapped.push(from..=to),
        }
    }
    mapped
}
