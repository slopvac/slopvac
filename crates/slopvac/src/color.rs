//! Colour resolution, decided once per run (docs/contract/policies.md, 2.1).

use crate::env::Env;

/// The value of `--color`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    /// Colour when stdout is a capable terminal.
    Auto,
    /// Always colour.
    Always,
    /// Never colour.
    Never,
}

impl ColorChoice {
    /// Parse the `--color` argument value.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }
}

fn non_empty(env: &dyn Env, name: &str) -> bool {
    env.var(name).is_some_and(|v| !v.is_empty())
}

/// Resolve whether to emit colour.
///
/// Order: an explicit `--color` (or `--no-color`) decides alone, with `auto` meaning the
/// terminal check. Without one: `NO_COLOR`, then
/// `CLICOLOR_FORCE` or `FORCE_COLOR`, then a terminal check with `TERM`.
#[must_use]
pub fn resolve(flag: Option<ColorChoice>, env: &dyn Env) -> bool {
    match flag {
        Some(ColorChoice::Always) => return true,
        Some(ColorChoice::Never) => return false,
        Some(ColorChoice::Auto) => return terminal_allows_color(env),
        None => {}
    }
    if non_empty(env, "NO_COLOR") {
        return false;
    }
    if non_empty(env, "CLICOLOR_FORCE") || non_empty(env, "FORCE_COLOR") {
        return true;
    }
    terminal_allows_color(env)
}

fn terminal_allows_color(env: &dyn Env) -> bool {
    env.stdout_is_terminal() && env.var("TERM").as_deref() != Some("dumb")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    #[test]
    fn precedence_table() {
        type Case<'a> = (Option<ColorChoice>, &'a [(&'a str, &'a str)], bool, bool);
        let cases: &[Case<'_>] = &[
            (Some(ColorChoice::Always), &[("NO_COLOR", "1")], false, true),
            (
                Some(ColorChoice::Never),
                &[("FORCE_COLOR", "1")],
                true,
                false,
            ),
            (Some(ColorChoice::Auto), &[("NO_COLOR", "1")], true, true),
            (
                Some(ColorChoice::Auto),
                &[("FORCE_COLOR", "1")],
                false,
                false,
            ),
            (
                Some(ColorChoice::Auto),
                &[("CLICOLOR_FORCE", "1")],
                false,
                false,
            ),
            (Some(ColorChoice::Auto), &[("TERM", "dumb")], true, false),
            (None, &[("NO_COLOR", "")], true, true),
            (
                None,
                &[("NO_COLOR", "1"), ("FORCE_COLOR", "1")],
                true,
                false,
            ),
            (None, &[("CLICOLOR_FORCE", "1")], false, true),
            (None, &[("FORCE_COLOR", "1")], false, true),
            (None, &[], true, true),
            (None, &[], false, false),
            (None, &[("TERM", "dumb")], true, false),
            (None, &[("TERM", "xterm")], true, true),
        ];
        for (flag, vars, terminal, want) in cases {
            let env = MapEnv::new(vars, *terminal);
            assert_eq!(
                resolve(*flag, &env),
                *want,
                "{flag:?} {vars:?} tty={terminal}"
            );
        }
    }
}
