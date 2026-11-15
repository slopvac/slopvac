//! Environment access behind a trait, so tests need no process-wide mutation.

use std::collections::HashMap;
use std::io::IsTerminal;

/// Read-only view of the process environment and terminal state.
pub trait Env {
    /// The value of a variable, or `None` when unset or not Unicode.
    fn var(&self, name: &str) -> Option<String>;
    /// Whether standard output is a terminal.
    fn stdout_is_terminal(&self) -> bool;
}

/// The real process environment.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemEnv;

impl Env for SystemEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn stdout_is_terminal(&self) -> bool {
        std::io::stdout().is_terminal()
    }
}

/// A fixed environment for tests.
#[derive(Debug, Clone, Default)]
pub struct MapEnv {
    /// Variables to expose.
    pub vars: HashMap<String, String>,
    /// What `stdout_is_terminal` reports.
    pub terminal: bool,
}

impl MapEnv {
    /// Build an environment from name and value pairs.
    #[must_use]
    pub fn new(vars: &[(&str, &str)], terminal: bool) -> Self {
        Self {
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            terminal,
        }
    }
}

impl Env for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn stdout_is_terminal(&self) -> bool {
        self.terminal
    }
}
