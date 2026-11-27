//! The model provider of `slopvac review`: the code plugin named under `[review] provider`,
//! asked through `provider/complete`.
//!
//! Nothing starts when the provider is built. `review` calls it only after the dry-run,
//! mechanical, offline, provider and reviewer-model gates, and the trust gate then refuses an
//! untrusted or revoked plugin before a process is spawned (policies.md N11 and N12). The
//! plugin is found, loaded and started through the same runner and loader as
//! `slopvac plugins`.

use std::path::{Path, PathBuf};

use slopvac_agents::AgentsError;
use slopvac_agents::review::{self, ModelProvider, ProviderReply, ProviderRequest, ReplyItem};
use slopvac_plugins::hooks::CompleteItem;
use slopvac_plugins::{CompleteError, HookKind, HostConfig, PluginError};

use crate::env::Env;
use crate::errors::CliError;
use crate::plugin_loader::InstalledPlugins;
use crate::redact;

/// The plugin named under `[review] provider`, resolved and started only when a call is due.
pub(crate) struct PluginProvider<'a> {
    id: String,
    offline: bool,
    env: &'a dyn Env,
    cwd: PathBuf,
}

impl<'a> PluginProvider<'a> {
    /// The provider plugin `id` of the project around `cwd`.
    pub(crate) fn new(id: String, offline: bool, env: &'a dyn Env, cwd: &Path) -> Self {
        Self {
            id,
            offline,
            env,
            cwd: cwd.to_path_buf(),
        }
    }

    /// A failure of the plugin layer under its own code, with every absolute path redacted and
    /// the text escaped once for the terminal.
    fn failure(&self, code: &'static str, text: &str) -> AgentsError {
        let shown = redact::redact_absolute(text, &self.cwd);
        AgentsError::new_coded(code, slopvac_sanitize::line(&shown))
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
            .map_err(|e| self.failure(e.code(), &e.to_string()))?;
        // Before the start: a plugin that declares no provider hook is not a model provider. The
        // first one it declares answers, as the declaration lists them.
        let hook = spec
            .declaration
            .hooks
            .iter()
            .find(|h| h.kind == HookKind::Provider.as_str())
            .map(|h| h.id.clone())
            .ok_or_else(review::not_a_provider)?;
        let host = runner
            .start(&plugin, spec, &HostConfig::default())
            .map_err(|e| self.failure(e.code(), &e.to_string()))?;
        let items: Vec<CompleteItem> = request
            .items
            .iter()
            .map(|item| CompleteItem {
                id: item.id.clone(),
                instruction: Some(item.instruction.to_owned()),
                text: item.text.clone(),
            })
            .collect();
        let answers = host.complete_checked(&hook, request.task, request.model, &items);
        let shutdown = host.shutdown();
        if !shutdown.clean() {
            tracing::debug!(plugin = %self.id, "the model provider did not shut down cleanly");
        }
        match answers {
            Ok(answers) => Ok(ProviderReply::new(
                answers
                    .into_iter()
                    .enumerate()
                    .map(|(index, answer)| match answer {
                        Ok(done) => ReplyItem::text(index, done.text, done.model),
                        Err(failed) => ReplyItem::failed(index, failed.code_name()),
                    })
                    .collect(),
            )),
            // The one stage that plugin-protocol.md section 5.3 reports under the review code.
            Err(CompleteError::Refused(_)) => Err(review::reply_invalid(items.len())),
            Err(other) => {
                let error = PluginError::from(other);
                Err(self.failure(error.slopvac_code(), &error.to_string()))
            }
        }
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
        crate::run::with_plugin_runner(self.offline, self.env, &self.cwd, |runner, loader| {
            Ok(self.ask(runner, loader, request))
        })
        .map_err(|e: CliError| AgentsError::new_coded(e.code, e.message))?
    }
}
