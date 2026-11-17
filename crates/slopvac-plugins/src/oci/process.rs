//! A credential helper or a `git` process, and everything it starts, killed as one unit.
//!
//! Unix: the helper leads its own process group, and the group is signalled
//! before the helper is reaped, so the group id cannot have been recycled.
//! Windows: `taskkill /T /F` on the helper's pid, as the plugin host does,
//! resolved from the system directory rather than `PATH`. A descendant that has
//! already left the helper's tree cannot be reached without job objects, which
//! need `unsafe`; that is deferred (see `kill_tree`).

use std::io;
use std::process::{Child, Command, ExitStatus};

/// A spawned helper that has not been reaped yet.
pub(crate) struct Contained {
    child: Child,
    /// Exit status already collected by a non-Unix poll.
    #[cfg(not(unix))]
    status: Option<ExitStatus>,
}

impl Contained {
    pub(crate) fn spawn(cmd: &mut Command) -> io::Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        Ok(Self {
            child: cmd.spawn()?,
            #[cfg(not(unix))]
            status: None,
        })
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }

    /// Whether the helper has exited. On Unix the exit stays unreaped, so the
    /// process group is still reserved and can be signalled safely.
    pub(crate) fn has_exited(&mut self) -> io::Result<bool> {
        #[cfg(unix)]
        {
            use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
            let pid = Pid::from_child(&self.child);
            let options = WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT;
            Ok(waitid(WaitId::Pid(pid), options)?.is_some())
        }
        #[cfg(not(unix))]
        {
            if self.status.is_none() {
                self.status = self.child.try_wait()?;
            }
            Ok(self.status.is_some())
        }
    }

    /// Collect the exit status of a helper that `has_exited`.
    pub(crate) fn reap(mut self) -> io::Result<ExitStatus> {
        #[cfg(not(unix))]
        if let Some(status) = self.status {
            return Ok(status);
        }
        self.child.wait()
    }

    /// Kill the helper and its descendants, then reap the helper.
    pub(crate) fn kill_tree(mut self) {
        #[cfg(unix)]
        {
            use rustix::process::{Pid, Signal, kill_process_group};
            // The helper is unreaped, so its pid still names the group. An
            // error means the group is already empty.
            let _ = kill_process_group(Pid::from_child(&self.child), Signal::KILL);
        }
        // Residual (native Windows, slopvac-8l4.18.8): once the helper has
        // exited, Windows no longer has its pid to name a tree, so a descendant
        // that outlived it and still holds stdout is not killed here. The
        // lookup still ends at its deadline and never joins the blocked
        // reader. Job objects close this gap.
        #[cfg(not(unix))]
        if self.status.is_none() {
            let _ = Command::new(taskkill_path())
                .args(["/T", "/F", "/PID", &self.child.id().to_string()])
                .output();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `%SystemRoot%\System32\taskkill.exe`, so a `taskkill` earlier on `PATH`
/// (or in the working directory) is never run in its place.
#[cfg(not(unix))]
fn taskkill_path() -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    std::path::Path::new(&root)
        .join("System32")
        .join("taskkill.exe")
}
