//! Bounded collection of the warnings, notes and blocking records a run reports.

use crate::Dropped;
use crate::messages::MigrateText;

/// Most records kept per list.
const MAX_MESSAGES: usize = 100;
/// Most characters kept per message or field.
const MAX_MESSAGE_CHARS: usize = 512;

/// A fresh string with the first [`MAX_MESSAGE_CHARS`] characters of `text`, and `…` when it
/// was cut. It never keeps the allocation of `text`, so a long input is freed.
fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_MESSAGE_CHARS) {
        Some((cut, _)) => {
            let mut out = String::with_capacity(cut + '…'.len_utf8());
            out.push_str(&text[..cut]);
            out.push('…');
            out
        }
        None => text.to_owned(),
    }
}

/// [`clip`] for an owned message. A short one is kept, trimmed to its length.
fn clip_owned(mut text: String) -> String {
    if text.char_indices().nth(MAX_MESSAGE_CHARS).is_some() {
        return clip(&text);
    }
    text.shrink_to_fit();
    text
}

/// What a [`Capped`] list holds, which names the messages in its closing line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Listed {
    /// Notes about changes the run made.
    Notes,
    /// Warnings about files the run did not check.
    Warnings,
}

/// A list of messages that keeps the first [`MAX_MESSAGES`], cuts each to
/// [`MAX_MESSAGE_CHARS`] characters, and counts the rest.
#[derive(Debug)]
pub(crate) struct Capped {
    listed: Listed,
    items: Vec<String>,
    suppressed: usize,
}

impl Capped {
    /// An empty list; `listed` names the messages in the suppressed line.
    pub(crate) fn new(listed: Listed) -> Self {
        Self {
            listed,
            items: Vec::new(),
            suppressed: 0,
        }
    }

    pub(crate) fn push(&mut self, message: String) {
        if self.items.len() >= MAX_MESSAGES {
            self.suppressed = self.suppressed.saturating_add(1);
            return;
        }
        self.items.push(clip_owned(message));
    }

    /// The kept messages, then one line that counts the suppressed ones.
    pub(crate) fn finish(self) -> Vec<String> {
        let mut items = self.items;
        if self.suppressed > 0 {
            let count = self.suppressed;
            items.push(
                match self.listed {
                    Listed::Notes => MigrateText::SuppressedNotes { count },
                    Listed::Warnings => MigrateText::SuppressedWarnings { count },
                }
                .to_string(),
            );
        }
        items
    }
}

/// The settings that stop a run, bounded the same way as [`Capped`]. Records past the cap are
/// only counted, so a hostile config cannot grow the list or force a copy of its keys.
#[derive(Debug, Default)]
pub(crate) struct Blockers {
    items: Vec<Dropped>,
    suppressed: usize,
}

impl Blockers {
    /// How many settings block, kept or not.
    pub(crate) fn total(&self) -> usize {
        self.items.len().saturating_add(self.suppressed)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.total() == 0
    }

    /// Records the setting at `line` of `file`. Past the cap it only counts, without copying.
    pub(crate) fn record(&mut self, file: &str, key: &str, line: usize, reason: &str) {
        if self.items.len() >= MAX_MESSAGES {
            self.suppressed = self.suppressed.saturating_add(1);
            return;
        }
        self.items
            .push(Dropped::new(clip(file), clip(key), line, clip(reason)));
    }

    /// Adds a record. A closing record of another capped list adds its count.
    pub(crate) fn push(&mut self, record: &Dropped) {
        if record.suppressed > 0 {
            self.suppressed = self.suppressed.saturating_add(record.suppressed);
            return;
        }
        self.record(&record.file, &record.key, record.line, &record.reason);
    }

    pub(crate) fn extend(&mut self, records: impl IntoIterator<Item = Dropped>) {
        for record in records {
            self.push(&record);
        }
    }

    /// The kept records, closed by one record that counts the suppressed ones. Empty when
    /// nothing blocks.
    pub(crate) fn finish(self) -> Vec<Dropped> {
        let mut items = self.items;
        if self.suppressed > 0 {
            items.push(Dropped::suppression(self.suppressed));
        }
        items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long() -> String {
        "x".repeat(100_000)
    }

    #[test]
    fn a_cut_message_does_not_keep_the_big_allocation() {
        let mut list = Capped::new(Listed::Warnings);
        list.push(long());
        let kept = list.finish();
        assert_eq!(kept[0].chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(kept[0].capacity() < 2048, "{}", kept[0].capacity());
    }

    #[test]
    fn a_cut_blocking_field_does_not_keep_the_big_allocation() {
        let mut list = Blockers::default();
        list.record(&long(), &long(), 3, &long());
        let kept = list.finish();
        for field in [&kept[0].file, &kept[0].key, &kept[0].reason] {
            assert!(field.capacity() < 2048, "{}", field.capacity());
        }
    }

    #[test]
    fn records_past_the_cap_are_counted_and_the_status_stays_blocking() {
        let mut list = Blockers::default();
        for _ in 0..(MAX_MESSAGES + 5) {
            list.record("a.toml", "k", 1, "r");
        }
        list.push(&Dropped::suppression(7));
        assert_eq!(list.total(), MAX_MESSAGES + 12);
        let kept = list.finish();
        assert_eq!(kept.len(), MAX_MESSAGES + 1);
        assert_eq!(kept[MAX_MESSAGES].suppressed, 12);
    }
}
