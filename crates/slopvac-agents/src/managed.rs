//! Whole files that setup owns: the skill and the reviewer agent.

use std::path::Path;

use crate::block::State;
use crate::error::AgentsError;
use crate::fsx::Project;

/// Ownership marker, the line right after the front matter. When its wording
/// changes, keep the old line in `KNOWN_MARKERS` so installed files stay manageable.
pub const MARKER: &str = "<!-- slopvac:managed; slopvac setup rewrites this file -->";
const KNOWN_MARKERS: [&str; 2] = [
    MARKER,
    "<!-- slopvac:managed; slopvac init or slopvac setup rewrites this file -->",
];

pub(crate) fn has_marker(text: &str) -> bool {
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
pub fn state(project: &Project, rel: &Path, expected: Option<&str>) -> State {
    let bytes = match project.read(rel) {
        Ok(Some(b)) => b,
        Ok(None) => {
            return if expected.is_some() {
                State::Missing
            } else {
                State::Disabled
            };
        }
        Err(_) => return State::Malformed,
    };
    let Ok(text) = String::from_utf8(bytes) else {
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
pub fn sync(project: &Project, rel: &Path, expected: Option<&str>) -> Result<bool, AgentsError> {
    let before = project.read(rel)?;
    if let Some(bytes) = &before
        && !String::from_utf8(bytes.clone()).is_ok_and(|t| has_marker(&t))
    {
        return Err(AgentsError::NotManaged(project.display(rel).into()));
    }
    match (expected, before) {
        (Some(e), before) => project.write_atomic(rel, before.as_deref(), e.as_bytes()),
        (None, None) => Ok(false),
        (None, Some(b)) => {
            if project.read(rel)?.as_deref() != Some(b.as_slice()) {
                return Err(AgentsError::Conflict(project.display(rel).into()));
            }
            project.remove_file(rel)?;
            Ok(true)
        }
    }
}
