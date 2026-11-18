//! fyx: language model providers authenticate on first use, not at startup.
//!
//! Upstream authenticates every provider when the native agent is created, which happens at
//! startup whenever a folder is open. With a bring-your-own key in the environment or keychain that
//! fetches model lists (Anthropic, Google, OpenRouter, …) and probes local servers, so Fyx would make
//! requests before the user touched any AI feature. Here the first look at the model list or the
//! first prompt starts it instead. Inline assists and commit messages already authenticate their
//! own provider when used.

use std::time::Duration;

use agent_client_protocol::v2::schema as acp_v2;
use futures::FutureExt as _;
use gpui::{App, Task};
use language_model::LanguageModelRegistry;

use crate::{LanguageModels, NativeAgentConnection};

/// How long a first prompt waits for the model lists before failing with "no model configured".
const FIRST_MODEL_TIMEOUT: Duration = Duration::from_secs(20);

impl LanguageModels {
    /// Authenticates the providers the first time models are needed. Returns whether this call
    /// started it.
    pub(crate) fn request_models(&mut self, cx: &mut App) -> bool {
        if std::mem::replace(&mut self.models_requested, true) {
            return false;
        }
        LanguageModelRegistry::global(cx)
            .update(cx, |registry, cx| registry.authenticate_providers(cx));
        true
    }
}

impl NativeAgentConnection {
    /// A first prompt to a thread without a model starts authentication and returns a task that
    /// resolves once the thread has a model (or after a timeout). `None` means send right away.
    pub(crate) fn wait_for_first_model(
        &self,
        session_id: &acp_v2::SessionId,
        cx: &mut App,
    ) -> Option<Task<()>> {
        let (thread, mut models_changed) = self.0.update(cx, |agent, cx| {
            let thread = agent.sessions.get(session_id)?.thread.clone();
            if thread.read(cx).model().is_some() || !agent.models.request_models(cx) {
                return None;
            }
            Some((thread, agent.models.watch()))
        })?;
        Some(cx.spawn(async move |cx| {
            let timeout = cx.background_executor().timer(FIRST_MODEL_TIMEOUT).fuse();
            futures::pin_mut!(timeout);
            while !thread.read_with(cx, |thread, _| thread.model().is_some()) {
                futures::select_biased! {
                    changed = models_changed.changed().fuse() => {
                        if changed.is_err() {
                            break;
                        }
                    }
                    _ = timeout => break,
                }
            }
        }))
    }
}

/// Opening the model picker is a first use: start loading the lists it shows.
pub(crate) fn on_list_models(connection: &NativeAgentConnection, cx: &mut App) {
    connection.0.update(cx, |agent, cx| {
        agent.models.request_models(cx);
    });
}
