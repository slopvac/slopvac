//! Whole files that setup owns: the skill and the reviewer agent.

use std::fs;
use std::path::Path;

use crate::block::{State, atomic_write};
use crate::error::AgentsError;

/// Ownership marker, the line right after the front matter. When its wording
/// changes, keep the old line in `KNOWN_MARKERS` so installed files stay manageable.
pub const MARKER: &str = "<!-- slopvac:managed; slopvac setup rewrites this file -->";
const KNOWN_MARKERS: [&str; 2] = [
    MARKER,
    "<!-- slopvac:managed; slopvac init or slopvac setup rewrites this file -->",
];

fn has_marker(text: &str) -> bool {
    let mut lines = text.split('\n').map(|l| l.trim_end_matches('\r'));
    if lines.next() != Some("---") {
        return false;
    }
    let rest: Vec<&str> = lines.collect();
    rest.iter()
        .position(|l| *l == "---")
        .and_then(|i| rest.get(i + 1))
        .is_some_and(|l| KNOWN_MARKERS.contains(l))
}

/// State of a managed file against `expected`; `None` means it is not wanted.
#[must_use]
pub fn state(path: &Path, expected: Option<&str>) -> State {
    if !path.exists() {
        return if expected.is_some() {
            State::Missing
        } else {
            State::Disabled
        };
    }
    let Some(text) = fs::read(path).ok().and_then(|b| String::from_utf8(b).ok()) else {
        return State::Malformed;
    };
    if !has_marker(&text) {
        return State::Malformed;
    }
    match expected {
        None => State::Stale,
        Some(e) => {
            let n = text.replace("\r\n", "\n").replace('\r', "\n");
            if n == e { State::Current } else { State::Stale }
        }
    }
}

/// Write `expected`, or delete the file when `None`. Refuses files setup did
/// not write. Removal also drops the parent directory when it is empty.
pub fn sync(path: &Path, expected: Option<&str>) -> Result<bool, AgentsError> {
    let before = if path.exists() {
        let bytes = fs::read(path).map_err(|e| AgentsError::io(path, e))?;
        if !String::from_utf8(bytes.clone()).is_ok_and(|t| has_marker(&t)) {
            return Err(AgentsError::NotManaged(path.into()));
        }
        Some(bytes)
    } else {
        None
    };
    match (expected, before) {
        (Some(e), before) => atomic_write(path, before.as_deref(), e.as_bytes()),
        (None, None) => Ok(false),
        (None, Some(b)) => {
            if fs::read(path).map_err(|e| AgentsError::io(path, e))? != b {
                return Err(AgentsError::Conflict(path.into()));
            }
            fs::remove_file(path).map_err(|e| AgentsError::io(path, e))?;
            if let Some(dir) = path.parent() {
                let _ = fs::remove_dir(dir);
            }
            Ok(true)
        }
    }
}
