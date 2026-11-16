//! The only way the mapper reads bytes.
//!
//! Source text and driver output are untrusted and can be large. Every read of either goes
//! through [`Src`] or [`Cur`], and each primitive adds the bytes it examines to a [`Meter`].
//! The meter has two counters:
//!
//! - `work` is charged against a budget. When it passes the budget the mapper stops matching
//!   and falls back to line precision.
//! - `scanned` counts every byte examined and never stops, so a loop that keeps reading
//!   after the budget is spent still shows in the total. The property tests bound it by a
//!   multiple of the input.
//!
//! The text itself is a private field of [`Src`] and [`Cur`], so mapping code cannot index,
//! slice or iterate it without being counted.

use crate::sub;

/// Counters for one mapping run.
#[derive(Debug, Clone)]
pub(super) struct Meter {
    work: usize,
    scanned: usize,
    budget: usize,
}

impl Meter {
    pub(super) const fn new(budget: usize) -> Self {
        Self {
            work: 0,
            scanned: 0,
            budget,
        }
    }

    /// A meter for a sub-run that shares this one's work so far.
    pub(super) fn child(&self) -> Self {
        Self {
            work: self.work,
            scanned: 0,
            budget: self.budget,
        }
    }

    /// Take back what a sub-run did.
    pub(super) const fn absorb(&mut self, child: &Self) {
        self.work = child.work;
        self.scanned = self.scanned.saturating_add(child.scanned);
    }

    /// Count `units` bytes examined. They are charged to the budget until it is spent.
    /// False once it is spent.
    pub(super) fn charge(&mut self, units: usize) -> bool {
        self.scanned = self.scanned.saturating_add(units);
        if self.work <= self.budget {
            self.work = self.work.saturating_add(units);
        }
        self.work <= self.budget
    }

    /// Count `units` bytes examined, or `None` once the budget is spent.
    pub(super) fn spend(&mut self, units: usize) -> Option<()> {
        self.charge(units).then_some(())
    }

    /// Count bytes examined by a step that must finish even when the budget is spent, such
    /// as trimming a line once for the fallback.
    pub(super) const fn scan(&mut self, units: usize) {
        self.scanned = self.scanned.saturating_add(units);
    }

    pub(super) const fn exhausted(&self) -> bool {
        self.work > self.budget
    }

    pub(super) const fn work(&self) -> usize {
        self.work
    }

    pub(super) const fn scanned(&self) -> usize {
        self.scanned
    }
}

/// A metered view of the source text (or of a view built from it).
#[derive(Clone, Copy)]
pub(super) struct Src<'a> {
    text: &'a str,
}

impl<'a> Src<'a> {
    pub(super) const fn new(text: &'a str) -> Self {
        Self { text }
    }

    pub(super) const fn len(self) -> usize {
        self.text.len()
    }

    /// Start offsets of the lines from `bom`: LF, CRLF and a lone CR end a line.
    pub(super) fn line_starts(self, m: &mut Meter, bom: usize) -> Vec<usize> {
        m.scan(self.text.len());
        let bytes = self.text.as_bytes();
        let mut starts = vec![bom];
        let mut i = bom;
        while let Some(&current) = bytes.get(i) {
            match current {
                b'\n' => starts.push(i + 1),
                b'\r' => {
                    if bytes.get(i + 1) == Some(&b'\n') {
                        i += 1;
                    }
                    starts.push(i + 1);
                }
                _ => {}
            }
            i += 1;
        }
        starts
    }

    /// End of the line that ends before `next_start`, without its terminator.
    ///
    /// `next_start` follows a line terminator, so it is at least 1. A smaller value gives
    /// `None`.
    pub(super) fn line_end_before(self, next_start: usize) -> Option<usize> {
        let bytes = self.text.as_bytes();
        let crlf = next_start
            .checked_sub(2)
            .and_then(|from| bytes.get(from..next_start))
            == Some(b"\r\n".as_slice());
        if crlf {
            next_start.checked_sub(2)
        } else {
            next_start.checked_sub(1)
        }
    }

    /// The text `a..b`. The bytes are charged: whatever the caller does with the slice is
    /// paid for.
    pub(super) fn slice(self, m: &mut Meter, a: usize, b: usize) -> &'a str {
        m.charge(b - a);
        sub(self.text, a..b)
    }

    /// Like [`Self::slice`], for a step that must not stop at the budget.
    pub(super) fn slice_free(self, m: &mut Meter, a: usize, b: usize) -> &'a str {
        m.scan(b - a);
        sub(self.text, a..b)
    }

    /// The character at `at`, if it starts before `hi`.
    pub(super) fn peek(self, m: &mut Meter, at: usize, hi: usize) -> Option<char> {
        if at >= hi {
            return None;
        }
        let c = sub(self.text, at..).chars().next()?;
        m.charge(c.len_utf8());
        Some(c)
    }

    /// The character before `at`.
    pub(super) fn before(self, m: &mut Meter, at: usize) -> Option<char> {
        let c = sub(self.text, ..at).chars().next_back()?;
        m.charge(c.len_utf8());
        Some(c)
    }

    /// The first character in `at..hi` that `pred` accepts, with its offset from `at`. The
    /// bytes examined are charged, found or not.
    pub(super) fn find(
        self,
        m: &mut Meter,
        at: usize,
        hi: usize,
        pred: impl Fn(char) -> bool,
    ) -> Option<(usize, char)> {
        let found = sub(self.text, at..hi)
            .char_indices()
            .find(|&(_, c)| pred(c));
        m.charge(found.map_or(hi - at, |(off, c)| off + c.len_utf8()));
        found
    }

    /// The end of the run of characters in `at..hi` that `pred` accepts.
    pub(super) fn skip_while(
        self,
        m: &mut Meter,
        at: usize,
        hi: usize,
        pred: impl Fn(char) -> bool,
    ) -> usize {
        let run = sub(self.text, at..hi)
            .char_indices()
            .find(|&(_, c)| !pred(c))
            .map_or(hi - at, |(off, _)| off);
        m.charge(run);
        at + run
    }

    /// Byte offsets of the character columns `x0..x1` of the line `a..b`. The columns are
    /// charged up front, so a wide window on a long line costs what it reads.
    pub(super) fn window(
        self,
        m: &mut Meter,
        a: usize,
        b: usize,
        x0: usize,
        x1: usize,
    ) -> Option<(usize, usize)> {
        // A spent budget reads nothing, so it counts nothing.
        if m.exhausted() || !m.charge(x1.min(b - a).saturating_add(1)) {
            return None;
        }
        let line = sub(self.text, a..b);
        let mut from = line.len();
        let mut to = line.len();
        for (i, (byte, _)) in line.char_indices().enumerate() {
            if i == x0 {
                from = byte;
            }
            if i == x1 {
                to = byte;
                break;
            }
        }
        Some((a + from.min(to), a + to))
    }

    /// The trimmed bounds of `a..b`: where its first and last non-whitespace characters
    /// are, or `None` for a blank range. Counts the whitespace it skips.
    pub(super) fn trimmed(self, m: &mut Meter, a: usize, b: usize) -> Option<(usize, usize)> {
        let slice = sub(self.text, a..b);
        let body = slice.trim_start();
        let lead = slice.len() - body.len();
        let tail = body.len() - body.trim_end().len();
        m.scan(lead + tail + 1);
        (!body.is_empty()).then(|| (a + lead, b - tail))
    }
}

/// A metered cursor over driver output.
#[derive(Clone, Copy)]
pub(super) struct Cur<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> Cur<'a> {
    pub(super) const fn new(text: &'a str) -> Self {
        Self { text, at: 0 }
    }

    pub(super) fn peek(self, m: &mut Meter) -> Option<char> {
        let c = sub(self.text, self.at..).chars().next()?;
        m.charge(c.len_utf8());
        Some(c)
    }

    pub(super) fn bump(&mut self, m: &mut Meter) -> Option<char> {
        let c = self.peek(m)?;
        self.at += c.len_utf8();
        Some(c)
    }

    /// Skip whitespace; true if any was skipped.
    pub(super) fn skip_ws(&mut self, m: &mut Meter) -> bool {
        let rest = sub(self.text, self.at..);
        let run = rest.len() - rest.trim_start().len();
        m.charge(run);
        self.at += run;
        run > 0
    }
}
