//! Project file access anchored to a directory handle.
//!
//! Every read and write goes through a `cap_std` directory opened once for the
//! project root. Path resolution happens inside the kernel relative to that
//! handle, so a directory swapped for a symlink between a check and its use
//! cannot lead outside the project (CWE-59, CWE-367). Absolute symlinks and links
//! that climb out of the root fail with [`AgentsError::UnsafeTarget`].

use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};

use crate::error::AgentsError;

/// Longest symlink chain setup follows for a steering file.
const MAX_LINKS: usize = 8;

/// A project directory opened as a capability.
pub struct Project {
    dir: Dir,
    root: PathBuf,
}

/// Check a project-relative target lexically: normal components only, no
/// `.git`, and a `.md` file name.
pub fn target(relative: &str) -> Result<PathBuf, AgentsError> {
    let bad = || AgentsError::UnsafeTarget(relative.to_owned());
    let mut clean = PathBuf::new();
    for part in Path::new(relative).components() {
        match part {
            Component::Normal(p) if p != ".git" => clean.push(p),
            _ => return Err(bad()),
        }
    }
    if clean.extension().and_then(|e| e.to_str()) != Some("md") {
        return Err(bad());
    }
    Ok(clean)
}

impl Project {
    /// Open the project root.
    pub fn open(root: &Path) -> Result<Self, AgentsError> {
        let dir = Dir::open_ambient_dir(root, ambient_authority())
            .map_err(|e| AgentsError::io(root, e))?;
        Ok(Self {
            dir,
            root: root.to_owned(),
        })
    }

    /// The path to show a user for `rel`.
    #[must_use]
    pub fn display(&self, rel: &Path) -> String {
        self.root.join(rel).display().to_string()
    }

    fn map(&self, rel: &Path, e: std::io::Error) -> AgentsError {
        if e.kind() == ErrorKind::PermissionDenied && e.to_string().contains("outside") {
            AgentsError::UnsafeTarget(self.display(rel))
        } else {
            AgentsError::io(self.root.join(rel), e)
        }
    }

    /// Follow a symlink at `rel` (inside the project) to the file it names.
    fn resolve(&self, rel: &Path) -> Result<PathBuf, AgentsError> {
        let mut current = rel.to_owned();
        for _ in 0..MAX_LINKS {
            let is_link = match self.dir.symlink_metadata(&current) {
                Ok(m) => m.file_type().is_symlink(),
                Err(e) if e.kind() == ErrorKind::NotFound => false,
                Err(e) => return Err(self.map(&current, e)),
            };
            if !is_link {
                return Ok(current);
            }
            let link = self
                .dir
                .read_link_contents(&current)
                .map_err(|e| self.map(&current, e))?;
            let mut next = current.parent().map(Path::to_path_buf).unwrap_or_default();
            for part in link.components() {
                match part {
                    Component::Normal(p) => next.push(p),
                    Component::CurDir => {}
                    Component::ParentDir if next.pop() => {}
                    _ => return Err(AgentsError::UnsafeTarget(self.display(rel))),
                }
            }
            current = next;
        }
        Err(AgentsError::UnsafeTarget(self.display(rel)))
    }

    /// The bytes of the file at `rel`, or `None` if it does not exist.
    pub fn read(&self, rel: &Path) -> Result<Option<Vec<u8>>, AgentsError> {
        let rel = self.resolve(rel)?;
        match self.dir.metadata(&rel) {
            Ok(m) if !m.is_file() => return Err(AgentsError::UnsafeTarget(self.display(&rel))),
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(self.map(&rel, e)),
        }
        self.dir.read(&rel).map(Some).map_err(|e| self.map(&rel, e))
    }

    /// Whether a file exists at `rel`.
    pub fn exists(&self, rel: &Path) -> Result<bool, AgentsError> {
        Ok(self.read(rel)?.is_some())
    }

    fn parent_dir(&self, rel: &Path, create: bool) -> Result<Dir, AgentsError> {
        let parent = rel.parent().filter(|p| !p.as_os_str().is_empty());
        let Some(parent) = parent else {
            return self.dir.try_clone().map_err(|e| self.map(rel, e));
        };
        if create {
            self.dir
                .create_dir_all(parent)
                .map_err(|e| self.map(parent, e))?;
        }
        self.dir.open_dir(parent).map_err(|e| self.map(parent, e))
    }

    /// Replace the file at `rel` with `after`, but only if it still holds
    /// `before`. Returns whether the file changed.
    pub fn write_atomic(
        &self,
        rel: &Path,
        before: Option<&[u8]>,
        after: &[u8],
    ) -> Result<bool, AgentsError> {
        if before == Some(after) {
            return Ok(false);
        }
        let rel = self.resolve(rel)?;
        if self.read(&rel)?.as_deref() != before {
            return Err(AgentsError::Conflict(self.root.join(&rel)));
        }
        let parent = self.parent_dir(&rel, true)?;
        let name = rel
            .file_name()
            .ok_or_else(|| AgentsError::UnsafeTarget(self.display(&rel)))?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut attempt = 0_u32;
        let (tmp_name, mut file) = loop {
            let tmp_name = format!(".slopvac-{}-{stamp}-{attempt}", std::process::id());
            match parent.open_with(&tmp_name, &options) {
                Ok(f) => break (tmp_name, f),
                Err(e) if e.kind() == ErrorKind::AlreadyExists && attempt < 100 => attempt += 1,
                Err(e) => return Err(self.map(&rel, e)),
            }
        };
        let mut finish = || -> std::io::Result<()> {
            file.write_all(after)?;
            if let Ok(meta) = parent.metadata(name) {
                file.set_permissions(meta.permissions())?;
            }
            file.sync_all()?;
            parent.rename(&tmp_name, &parent, name)
        };
        finish().map_err(|e| {
            let _ = parent.remove_file(&tmp_name);
            self.map(&rel, e)
        })?;
        Ok(true)
    }

    /// Delete the file at `rel` and its parent directory if that is now empty.
    pub fn remove_file(&self, rel: &Path) -> Result<(), AgentsError> {
        let rel = self.resolve(rel)?;
        let parent = self.parent_dir(&rel, false)?;
        let name = rel
            .file_name()
            .ok_or_else(|| AgentsError::UnsafeTarget(self.display(&rel)))?;
        parent.remove_file(name).map_err(|e| self.map(&rel, e))?;
        if let Some(dir) = rel.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = self.dir.remove_dir(dir);
        }
        Ok(())
    }
}
