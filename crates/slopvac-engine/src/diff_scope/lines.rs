//! Translate git's line numbers into the numbers findings carry.
//!
//! Git counts a line per LF. Findings come from `SourceIndex`, where LF, CRLF and a CR
//! that does not precede LF each end a line. The two agree unless the file holds a lone
//! CR, which splits one git line into several source lines.
//!
//! The file is read in fixed chunks and never held whole. Only the requested ranges are
//! kept, so memory does not grow with the file or its line count.

use std::io::{self, Read};
use std::ops::RangeInclusive;

/// Where the translation stands in the file.
struct Walk<'r> {
    ranges: &'r [RangeInclusive<u32>],
    /// The range being matched next.
    next: usize,
    /// The git line (LF count + 1) and the source line the walk is on.
    git: u32,
    source: u32,
    /// The source line where the current git line began.
    first: u32,
    /// The source line where the open range began.
    from: Option<u32>,
    /// A CR was the last byte seen; the next byte says whether it is a lone CR.
    pending_cr: bool,
    out: Vec<RangeInclusive<u32>>,
}

impl<'r> Walk<'r> {
    fn new(ranges: &'r [RangeInclusive<u32>]) -> Self {
        let mut walk = Self {
            ranges,
            next: 0,
            git: 1,
            source: 1,
            first: 1,
            from: None,
            pending_cr: false,
            out: Vec::with_capacity(ranges.len()),
        };
        walk.begin_git_line();
        walk
    }

    fn begin_git_line(&mut self) {
        if self.from.is_none()
            && self
                .ranges
                .get(self.next)
                .is_some_and(|r| *r.start() == self.git)
        {
            self.from = Some(self.first);
        }
    }

    /// The git line ended on source line `last`: close the range that ends here.
    fn end_git_line(&mut self, last: u32) {
        if let (Some(from), Some(range)) = (self.from, self.ranges.get(self.next))
            && *range.end() <= self.git
        {
            self.push(from, last);
            self.from = None;
            self.next += 1;
        }
    }

    fn push(&mut self, from: u32, to: u32) {
        match self.out.last_mut() {
            Some(last) if from <= last.end().saturating_add(1) => {
                *last = *last.start()..=(*last.end()).max(to);
            }
            _ => self.out.push(from..=to),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        for &byte in chunk {
            if self.pending_cr {
                self.pending_cr = false;
                if byte != b'\n' {
                    // The CR stood alone: it ended a source line.
                    self.source = self.source.saturating_add(1);
                }
            }
            match byte {
                b'\n' => {
                    self.end_git_line(self.source);
                    self.source = self.source.saturating_add(1);
                    self.git = self.git.saturating_add(1);
                    self.first = self.source;
                    self.begin_git_line();
                }
                b'\r' => self.pending_cr = true,
                _ => {}
            }
        }
    }

    fn finish(mut self) -> Vec<RangeInclusive<u32>> {
        if self.pending_cr {
            self.source = self.source.saturating_add(1);
        }
        // A range that runs past the last line ends on the last source line.
        if let Some(from) = self.from {
            self.push(from, self.source);
        }
        self.out
    }
}

/// Map `ranges` (git line numbers, sorted, merged) to source line numbers while reading
/// `reader`. Returns the ranges and how many bytes were read.
///
/// A whole-file range stays whole without reading. A range that starts past the last
/// line is dropped.
///
/// # Errors
///
/// Any read error.
pub(super) fn to_source_lines(
    reader: &mut impl Read,
    ranges: &[RangeInclusive<u32>],
) -> io::Result<(Vec<RangeInclusive<u32>>, u64)> {
    if ranges.iter().any(|r| *r.end() == u32::MAX) {
        return Ok((ranges.to_vec(), 0));
    }
    let mut walk = Walk::new(ranges);
    let mut chunk = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        total += u64::try_from(n).map_err(std::io::Error::other)?;
        walk.feed(chunk.get(..n).unwrap_or_default());
    }
    Ok((walk.finish(), total))
}
