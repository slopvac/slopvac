//! Bounded collection of the warnings, notes and blocking records a run reports.

use crate::Dropped;

/// Most records kept per list.
const MAX_MESSAGES: usize = 100;
/// Most characters kept per message or field.
const MAX_MESSAGE_CHARS: usize = 512;

/// `text` cut to [`MAX_MESSAGE_CHARS`] characters, with `…` when it was cut.
fn clip(mut text: String) -> String {
    if let Some((cut, _)) = text.char_indices().nth(MAX_MESSAGE_CHARS) {
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// A list of messages that keeps the first [`MAX_MESSAGES`], cuts each to
/// [`MAX_MESSAGE_CHARS`] characters, and counts the rest.
#[derive(Debug)]
pub(crate) struct Capped {
    what: &'static str,
    items: Vec<String>,
    suppressed: usize,
}

impl Capped {
    /// An empty list; `what` names the messages in the suppressed line.
    pub(crate) fn new(what: &'static str) -> Self {
        Self {
            what,
            items: Vec::new(),
            suppressed: 0,
        }
    }

    pub(crate) fn push(&mut self, message: String) {
        if self.items.len() >= MAX_MESSAGES {
            self.suppressed = self.suppressed.saturating_add(1);
            return;
        }
        self.items.push(clip(message));
    }

    /// The kept messages, then one line that counts the suppressed ones.
    pub(crate) fn finish(self) -> Vec<String> {
        let mut items = self.items;
        if self.suppressed > 0 {
            items.push(format!("{} more {} suppressed", self.suppressed, self.what));
        }
        items
    }
}

/// The settings that stop a run, bounded the same way as [`Capped`].
#[derive(Debug, Default)]
pub(crate) struct Blockers {
    items: Vec<Dropped>,
    suppressed: usize,
}

impl Blockers {
    pub(crate) fn push(&mut self, mut record: Dropped) {
        if self.items.len() >= MAX_MESSAGES {
            self.suppressed = self.suppressed.saturating_add(1);
            return;
        }
        record.file = clip(record.file);
        record.key = clip(record.key);
        record.reason = clip(record.reason);
        self.items.push(record);
    }

    pub(crate) fn extend(&mut self, records: impl IntoIterator<Item = Dropped>) {
        for record in records {
            self.push(record);
        }
    }

    /// The kept records, closed by one record that counts the suppressed ones. Empty when
    /// nothing blocks.
    pub(crate) fn finish(self) -> Vec<Dropped> {
        let mut items = self.items;
        if self.suppressed > 0 {
            items.push(Dropped {
                file: String::new(),
                key: String::new(),
                line: 0,
                reason: format!("{} more blocking settings suppressed", self.suppressed),
            });
        }
        items
    }
}
