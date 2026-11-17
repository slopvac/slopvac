//! Bounded collection of the warnings and notes a run reports.

/// Most messages kept per list.
const MAX_MESSAGES: usize = 100;
/// Most characters kept per message.
const MAX_MESSAGE_CHARS: usize = 512;

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
        let mut message = message;
        if let Some((cut, _)) = message.char_indices().nth(MAX_MESSAGE_CHARS) {
            message.truncate(cut);
            message.push('…');
        }
        self.items.push(message);
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
