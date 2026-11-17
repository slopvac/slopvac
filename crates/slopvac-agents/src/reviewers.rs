//! Installed reviewer agent files and the model each one sets.

use std::path::{Path, PathBuf};

use crate::content::INHERIT;
use crate::error::AgentsError;
use crate::fsx::Project;
use crate::harness::HARNESSES;
use crate::managed::has_marker;

/// One reviewer agent file that setup wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewerAgent {
    /// The harness name.
    pub harness: &'static str,
    /// The file, relative to the project root.
    pub path: PathBuf,
    /// The `model:` the file sets. `None` when the line is absent, commented out or
    /// `inherit`, so the harness picks the model.
    pub model: Option<String>,
}

/// Find the reviewer agent files under `root` that slopvac manages.
///
/// A file without the slopvac marker belongs to the user and is skipped, as is a
/// file that is not UTF-8.
///
/// # Errors
/// The project root is unsafe or a file cannot be read.
pub fn reviewer_agents(root: &Path) -> Result<Vec<ReviewerAgent>, AgentsError> {
    let project = Project::open(root)?;
    let mut found = Vec::new();
    for h in &HARNESSES {
        let Some(path) = h.agent_path()? else {
            continue;
        };
        let Some(bytes) = project.read(&path)? else {
            continue;
        };
        let Ok(text) = String::from_utf8(bytes) else {
            continue;
        };
        if !has_marker(&text) {
            continue;
        }
        found.push(ReviewerAgent {
            harness: h.name,
            path,
            model: front_matter_model(&text),
        });
    }
    Ok(found)
}

fn front_matter_model(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()? != "---" {
        return None;
    }
    lines
        .take_while(|l| *l != "---")
        .find_map(|l| l.strip_prefix("model:"))
        .map(|v| v.trim().trim_matches('"').to_owned())
        .filter(|v| !v.is_empty() && v != INHERIT)
}
