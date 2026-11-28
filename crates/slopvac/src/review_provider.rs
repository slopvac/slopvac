//! The model provider of `slopvac review`: the code plugin named under `[review] provider`,
//! asked through `provider/complete`.
//!
//! Nothing starts when the provider is built. `review` calls it only after the dry-run,
//! mechanical, offline, provider and reviewer-model gates, and the trust gate then refuses an
//! untrusted or revoked plugin before a process is spawned (policies.md N11 and N12). The
//! plugin is found, loaded and started through the same runner and loader as
//! `slopvac plugins`, built from the project config that `review` resolved once.

use std::path::{Path, PathBuf};

use slopvac_agents::AgentsError;
use slopvac_agents::review::{self, ModelProvider, ProviderReply, ProviderRequest, ReplyItem};
use slopvac_config::LoadedConfig;
use slopvac_plugins::{CompleteError, HookKind, HostConfig, PluginError, Shutdown};

use crate::env::Env;
use crate::errors::CliError;
use crate::plugin_loader::{InstalledPlugins, LoadRefusal};
use crate::redact;

/// Which loader step refused the provider plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadStage {
    /// The plugin is not locked, not installed or changed since sync.
    Synced,
    /// The installed files break the plugin contract.
    Code,
}

/// Why the provider plugin could not be loaded, kept typed as the cause of the review error:
/// the step, the stable code, and the text with every absolute path redacted and escaped once
/// for the terminal, so the error chain names no path (policies.md section 1.7).
#[derive(Debug, thiserror::Error)]
#[error("{text}")]
pub(crate) struct LoadCause {
    stage: LoadStage,
    code: &'static str,
    text: String,
}

impl LoadCause {
    /// The cause of `refusal`, with paths shown relative to `cwd`.
    fn new(refusal: &LoadRefusal, cwd: &Path) -> Self {
        let stage = match refusal {
            LoadRefusal::Synced(_) => LoadStage::Synced,
            LoadRefusal::Code(_) => LoadStage::Code,
        };
        let shown = redact::redact_absolute(&refusal.to_string(), cwd);
        Self {
            stage,
            code: refusal.code(),
            text: slopvac_sanitize::line(&shown),
        }
    }

    /// The loader step that refused the plugin.
    pub(crate) fn stage(&self) -> LoadStage {
        self.stage
    }

    /// The stable code of the refusal.
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }
}

/// The plugin named under `[review] provider`, resolved and started only when a call is due.
pub(crate) struct PluginProvider<'a> {
    id: String,
    offline: bool,
    env: &'a dyn Env,
    cwd: PathBuf,
    config: Option<&'a LoadedConfig>,
}

impl<'a> PluginProvider<'a> {
    /// The provider plugin `id` of the project around `cwd`, with the project config that
    /// `review` already resolved. The runner reads the same snapshot, never the files again.
    pub(crate) fn new(
        id: String,
        offline: bool,
        env: &'a dyn Env,
        cwd: &Path,
        config: Option<&'a LoadedConfig>,
    ) -> Self {
        Self {
            id,
            offline,
            env,
            cwd: cwd.to_path_buf(),
            config,
        }
    }

    /// A failure of the plugin layer under its own code, with every absolute path redacted and
    /// the text escaped once for the terminal.
    fn failure(&self, code: &'static str, text: &str) -> AgentsError {
        let shown = redact::redact_absolute(text, &self.cwd);
        AgentsError::new_coded(code, slopvac_sanitize::line(&shown))
    }

    /// A failure of the call itself. The host's text is shown, except where it can carry what
    /// the plugin sent ([`PluginError::text_is_host_only`]). That text can quote the document
    /// (policies.md section 1.7), so only the code shows.
    fn call_failure(&self, error: &PluginError) -> AgentsError {
        if error.text_is_host_only() {
            self.failure(error.slopvac_code(), &error.to_string())
        } else {
            review::request_failed(error.slopvac_code())
        }
    }

    /// A refusal of the loader, under its own code, with its typed cause kept.
    fn load_failure(&self, refusal: &LoadRefusal) -> AgentsError {
        let cause = LoadCause::new(refusal, &self.cwd);
        AgentsError::new_coded(cause.code, cause.text.clone()).with_source(cause)
    }

    fn ask(
        &self,
        runner: &slopvac_plugins::Runner<'_>,
        loader: &InstalledPlugins<'_>,
        request: &ProviderRequest<'_>,
    ) -> Result<ProviderReply, AgentsError> {
        let locked = runner
            .locked_code(&self.id)
            .map_err(|e| self.failure(e.code(), &e.to_string()))?;
        let (plugin, spec) = loader
            .try_load(&locked)
            .map_err(|e| self.load_failure(&e))?;
        // Before the start: a plugin that declares no provider hook is not a model provider.
        if !spec
            .declaration
            .hooks
            .iter()
            .any(|h| h.kind == HookKind::Provider.as_str())
        {
            return Err(review::not_a_provider());
        }
        let host = runner
            .start(&plugin, spec, &HostConfig::default())
            .map_err(|e| self.failure(e.code(), &e.to_string()))?;
        // After the handshake: the first provider hook the plugin offered answers.
        let hook = host
            .negotiated()
            .hooks
            .iter()
            .find(|h| h.kind == HookKind::Provider.as_str())
            .map(|h| h.id.clone());
        // The plan's own request goes out as it is (threat model TM-19).
        let answers = hook
            .as_deref()
            .map(|hook| host.complete_checked(hook, request.call));
        let shutdown = host.shutdown();
        // A failure of the request comes first, then an item the provider failed; the shutdown
        // can only add to a reply that is clean.
        let answers = match answers {
            None => return Err(review::not_a_provider()),
            Some(Err(CompleteError::Refused(_))) => {
                // The one stage that plugin-protocol.md section 5.3 reports under the review code.
                return Err(review::reply_invalid(request.items.len()));
            }
            Some(Err(other)) => return Err(self.call_failure(&PluginError::from(other))),
            Some(Ok(answers)) => answers,
        };
        if let Some(code) = shutdown_code(&shutdown)
            && answers.iter().all(Result::is_ok)
        {
            return Err(review::shutdown_failed(code));
        }
        Ok(ProviderReply::new(
            answers
                .into_iter()
                .enumerate()
                .map(|(index, answer)| match answer {
                    Ok(done) => ReplyItem::text(index, done.text, done.model),
                    Err(failed) => ReplyItem::failed(index, failed.code_name()),
                })
                .collect(),
        ))
    }
}

/// The host code for a shutdown that was not clean, or `None` for a clean one: a protocol
/// violation, the code of the failure that kept the plugin from acknowledging `shutdown` (an
/// error answer, an exit, a timeout), or an exit that was not the clean status 0 after
/// `shutdown` and `exit` (plugin-protocol.md sections 3 and 6.3).
fn shutdown_code(shutdown: &Shutdown) -> Option<&'static str> {
    if shutdown.clean() {
        None
    } else if shutdown.violation.is_some() {
        Some("plugin-protocol-violation")
    } else if let Some(refused) = &shutdown.refused {
        Some(refused.slopvac_code())
    } else {
        Some("plugin-exited")
    }
}

impl ModelProvider for PluginProvider<'_> {
    fn name(&self) -> &str {
        &self.id
    }

    fn model(&self) -> Option<&str> {
        None
    }

    fn review(&self, request: &ProviderRequest<'_>) -> Result<ProviderReply, AgentsError> {
        crate::run::with_plugin_runner(
            self.offline,
            self.env,
            &self.cwd,
            self.config,
            |runner, loader| Ok(self.ask(runner, loader, request)),
        )
        .map_err(|e: CliError| AgentsError::new_coded(e.code, e.message))?
    }
}

#[cfg(test)]
mod tests {
    use slopvac_packs::SyncedError;

    use super::*;

    /// A loader refusal reaches the review error with its cause kept typed: the step and the
    /// code, beside the same text.
    #[test]
    fn a_load_refusal_keeps_its_typed_cause() {
        let env = crate::env::MapEnv::new(&[], false);
        let provider = PluginProvider::new("p".to_owned(), true, &env, Path::new("/project"), None);
        let refusal = LoadRefusal::Synced(SyncedError::NotInstalled { id: "p".to_owned() });

        let error = provider.load_failure(&refusal);

        assert_eq!(error.code(), refusal.code());
        let cause = std::error::Error::source(&error)
            .and_then(|s| s.downcast_ref::<LoadCause>())
            .expect("a typed load cause");
        assert_eq!(cause.stage(), LoadStage::Synced);
        assert_eq!(cause.code(), refusal.code());
        assert_eq!(cause.to_string(), error.to_string());
    }
}
