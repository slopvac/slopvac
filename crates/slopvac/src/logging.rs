//! Opt-in logging to stderr (docs/contract/policies.md, 1.4).
//!
//! Logging stays off until `SLOPVAC_LOG` names a level. Call sites log only
//! rule IDs, pack IDs, digests, durations, counts and redacted paths. They
//! never log document text, matches, front matter values or terms.

use crate::env::Env;
use tracing::level_filters::LevelFilter;

/// Parse the `SLOPVAC_LOG` value. Unknown values leave logging off.
#[must_use]
pub fn level_from(env: &dyn Env) -> Option<LevelFilter> {
    let raw = env.var("SLOPVAC_LOG")?;
    let level: LevelFilter = raw.trim().parse().ok()?;
    (level != LevelFilter::OFF).then_some(level)
}

/// Install the stderr subscriber when `SLOPVAC_LOG` asks for one.
pub fn init(env: &dyn Env, ansi: bool) {
    if let Some(level) = level_from(env) {
        // A second init in one process (tests) is harmless.
        #[expect(
            clippy::let_underscore_must_use,
            reason = "a second init in one process (tests) is harmless"
        )]
        let _ = tracing_subscriber::fmt()
            .with_max_level(level)
            .with_writer(std::io::stderr)
            .with_ansi(ansi)
            .with_target(false)
            .try_init();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    #[test]
    fn unset_and_off_disable_logging() {
        assert_eq!(level_from(&MapEnv::default()), None);
        assert_eq!(
            level_from(&MapEnv::new(&[("SLOPVAC_LOG", "off")], false)),
            None
        );
        assert_eq!(
            level_from(&MapEnv::new(&[("SLOPVAC_LOG", "nonsense")], false)),
            None
        );
    }

    #[test]
    fn trace_is_accepted() {
        assert_eq!(
            level_from(&MapEnv::new(&[("SLOPVAC_LOG", "trace")], false)),
            Some(LevelFilter::TRACE)
        );
    }
}
