//! Sticky offline mode (docs/contract/policies.md, 1.3).
//!
//! Any source that turns offline on wins. No source can turn it off.

use crate::env::Env;

/// Whether `SLOPVAC_OFFLINE` asks for offline mode.
///
/// Any non-empty value other than an explicit false word counts. A typo
/// therefore errs towards staying offline.
#[must_use]
pub fn env_requests_offline(env: &dyn Env) -> bool {
    env.var("SLOPVAC_OFFLINE").is_some_and(|v| {
        let v = v.trim().to_ascii_lowercase();
        !(v.is_empty() || matches!(v.as_str(), "0" | "false" | "no" | "off"))
    })
}

/// Combine the flag, the environment and the `[network] offline` config value.
#[must_use]
pub fn resolve(flag: bool, env: &dyn Env, config: Option<bool>) -> bool {
    flag || env_requests_offline(env) || config == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    #[test]
    fn env_beats_config_false() {
        let env = MapEnv::new(&[("SLOPVAC_OFFLINE", "1")], false);
        assert!(resolve(false, &env, Some(false)));
    }

    #[test]
    fn config_true_alone_is_offline() {
        assert!(resolve(false, &MapEnv::default(), Some(true)));
    }

    #[test]
    fn nothing_set_is_online() {
        assert!(!resolve(false, &MapEnv::default(), None));
    }

    #[test]
    fn false_words_do_not_enable() {
        for word in ["", "0", "false", "No", "OFF"] {
            let env = MapEnv::new(&[("SLOPVAC_OFFLINE", word)], false);
            assert!(!env_requests_offline(&env), "{word:?}");
        }
    }

    #[test]
    fn unrecognised_value_errs_offline() {
        let env = MapEnv::new(&[("SLOPVAC_OFFLINE", "yes please")], false);
        assert!(env_requests_offline(&env));
    }
}
