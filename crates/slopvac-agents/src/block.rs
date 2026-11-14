//! Managed steering blocks and whole-file writes.

use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use pulldown_cmark::{Event, Parser, Tag};

use crate::error::AgentsError;

/// Opening marker of the managed block.
pub const BEGIN: &str = "<!-- slopvac:begin -->";
/// Closing marker of the managed block.
pub const END: &str = "<!-- slopvac:end -->";

/// State of a managed block or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Not installed.
    Missing,
    /// Installed and equal to what setup would write.
    Current,
    /// Installed but different.
    Stale,
    /// Present but not safe for setup to touch.
    Malformed,
    /// Not wanted and not installed.
    Disabled,
}

impl State {
    /// The word printed by `setup --check`.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Current => "current",
            Self::Stale => "stale",
            Self::Malformed => "malformed",
            Self::Disabled => "disabled",
        }
    }
}

/// Resolve `relative` under `root`; refuse `..`, `.git`, non-Markdown targets
/// and symlinks that leave the project.
pub fn safe_target(root: &Path, relative: &str) -> Result<PathBuf, AgentsError> {
    let bad = || AgentsError::UnsafeTarget(relative.to_owned());
    let wanted = Path::new(relative);
    let mut clean = PathBuf::new();
    for part in wanted.components() {
        match part {
            Component::Normal(p) if p != ".git" => clean.push(p),
            _ => return Err(bad()),
        }
    }
    if clean.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(bad());
    }
    let project = root.canonicalize().map_err(|e| AgentsError::io(root, e))?;
    let target = project.join(&clean);
    let mut probe = target.as_path();
    while !probe.exists() {
        probe = probe.parent().ok_or_else(bad)?;
    }
    let real = probe
        .canonicalize()
        .map_err(|e| AgentsError::io(probe, e))?;
    if !real.starts_with(&project) {
        return Err(bad());
    }
    if target.exists() && !target.is_file() {
        return Err(bad());
    }
    Ok(target)
}

fn read_utf8(path: &Path) -> Result<Option<(Vec<u8>, String)>, AgentsError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|e| AgentsError::io(path, e))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| AgentsError::NotUtf8(path.into()))?;
    Ok(Some((bytes, text)))
}

/// Byte range of the managed section, ignoring markers inside code blocks.
fn section(text: &str, path: &Path) -> Result<Option<(usize, usize)>, AgentsError> {
    let fenced: Vec<std::ops::Range<usize>> = Parser::new(text)
        .into_offset_iter()
        .filter_map(|(event, range)| match event {
            Event::Start(Tag::CodeBlock(_)) => Some(range),
            _ => None,
        })
        .collect();
    let mut found: Vec<(&str, usize, usize)> = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let inside = fenced.iter().any(|r| r.contains(&offset));
        let value = line.trim().trim_start_matches('\u{feff}');
        if !inside && (value == BEGIN || value == END) {
            let bom = usize::from(line.starts_with('\u{feff}')) * '\u{feff}'.len_utf8();
            found.push((value, offset + bom, offset + line.len()));
        }
        offset += line.len();
    }
    match found.as_slice() {
        [] => Ok(None),
        [(BEGIN, start, _), (END, _, end)] => Ok(Some((*start, *end))),
        _ => Err(AgentsError::MalformedMarkers(path.into())),
    }
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\n') {
        "\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

fn native(value: &str, nl: &str) -> String {
    value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', nl)
}

/// Write `after` atomically if `before` still matches the disk. Returns whether
/// the file changed.
pub fn atomic_write(path: &Path, before: Option<&[u8]>, after: &[u8]) -> Result<bool, AgentsError> {
    if before == Some(after) {
        return Ok(false);
    }
    let actual = if path.exists() {
        Some(fs::read(path).map_err(|e| AgentsError::io(path, e))?)
    } else {
        None
    };
    if actual.as_deref() != before {
        return Err(AgentsError::Conflict(path.into()));
    }
    let dir = path
        .parent()
        .ok_or_else(|| AgentsError::UnsafeTarget(path.display().to_string()))?;
    fs::create_dir_all(dir).map_err(|e| AgentsError::io(dir, e))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".slopvac-")
        .tempfile_in(dir)
        .map_err(|e| AgentsError::io(dir, e))?;
    tmp.write_all(after).map_err(|e| AgentsError::io(path, e))?;
    if path.exists() {
        let perms = fs::metadata(path)
            .map_err(|e| AgentsError::io(path, e))?
            .permissions();
        fs::set_permissions(tmp.path(), perms).map_err(|e| AgentsError::io(path, e))?;
    }
    tmp.persist(path)
        .map_err(|e| AgentsError::io(path, e.error))?;
    Ok(true)
}

/// State of the managed block in `path` against `block`.
#[must_use]
pub fn block_state(path: &Path, block: &str) -> State {
    let text = match read_utf8(path) {
        Ok(Some((_, text))) => text,
        Ok(None) => return State::Missing,
        Err(_) => return State::Malformed,
    };
    match section(&text, path) {
        Err(_) => State::Malformed,
        Ok(None) => State::Missing,
        Ok(Some((s, e))) => {
            let installed = native(text[s..e].trim(), "\n");
            if installed == block.trim() {
                State::Current
            } else {
                State::Stale
            }
        }
    }
}

/// Insert or replace the managed block, leaving surrounding text alone.
pub fn update_block(path: &Path, block: &str, preamble: &str) -> Result<bool, AgentsError> {
    let existing = read_utf8(path)?;
    let (before, text) = match &existing {
        Some((b, t)) => (Some(b.as_slice()), t.as_str()),
        None => (None, ""),
    };
    let nl = newline(text);
    let body = format!("{}{nl}", native(block.trim_end_matches(['\r', '\n']), nl));
    let updated = if let Some((s, e)) = section(text, path)? {
        format!("{}{body}{}", &text[..s], &text[e..])
    } else {
        let mut prefix = text.trim_end_matches(['\r', '\n']).to_owned();
        if prefix.is_empty() && !preamble.is_empty() {
            prefix = native(preamble.trim_end_matches(['\r', '\n']), nl);
        }
        if prefix.is_empty() {
            body
        } else {
            format!("{prefix}{nl}{nl}{body}")
        }
    };
    atomic_write(path, before, updated.as_bytes())
}

/// Remove only the managed block.
pub fn remove_block(path: &Path) -> Result<bool, AgentsError> {
    let Some((bytes, text)) = read_utf8(path)? else {
        return Ok(false);
    };
    let Some((s, e)) = section(&text, path)? else {
        return Ok(false);
    };
    let nl = newline(&text);
    let left = text[..s].trim_end_matches(['\r', '\n']);
    let right = text[e..].trim_start_matches(['\r', '\n']);
    let updated = match (left.is_empty(), right.is_empty()) {
        (false, false) => format!("{left}{nl}{nl}{right}"),
        (false, true) => format!("{left}{nl}"),
        _ => right.to_owned(),
    };
    atomic_write(path, Some(&bytes), updated.as_bytes())
}
