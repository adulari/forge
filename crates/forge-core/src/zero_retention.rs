//! The provider-level backstop for `[privacy] zero_retention` (docs/features/zero-retention.md).
//!
//! The router already drops retaining providers, but several callers pick a model without it: an
//! in-session `/model` pin, the architect planner, a side call under a pin, the failover last
//! resort, and `forge api`'s exact-model requests. Wrapping the provider means no request can
//! reach a retaining provider through any of them, and every request that does go out carries the
//! provider's opt-out where one exists.

use std::sync::Arc;

use forge_config::Config;
use forge_provider::{
    CompletionOptions, EventSink, ModelResponse, Provider, ProviderError, ToolSpec,
};
use forge_types::Message;

const REFUSAL_PREFIX: &str = "zero-retention mode refused to send this request: ";

/// `provider` unchanged when the mode is off; otherwise a guard over it.
pub fn guard(provider: Arc<dyn Provider>, config: &Config) -> Arc<dyn Provider> {
    if !config.privacy.zero_retention {
        return provider;
    }
    Arc::new(ZeroRetentionProvider {
        inner: provider,
        config: config.clone(),
    })
}

/// Whether `error` is this guard's refusal: a policy decision about this session's privacy
/// settings, not evidence about the model, so it must never bench the model in the shared store.
pub fn is_refusal(error: &ProviderError) -> bool {
    matches!(error, ProviderError::Request(message) if message.starts_with(REFUSAL_PREFIX))
}

struct ZeroRetentionProvider {
    inner: Arc<dyn Provider>,
    config: Config,
}

impl ZeroRetentionProvider {
    fn refuse(&self, model: &str) -> Result<(), ProviderError> {
        match forge_mesh::privacy::retention_block(&self.config, model) {
            Some(reason) => Err(ProviderError::Request(format!(
                "{REFUSAL_PREFIX}{model} ({reason})"
            ))),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ZeroRetentionProvider {
    async fn complete(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.complete_with(
            model,
            messages,
            tools,
            &CompletionOptions::default(),
            on_event,
        )
        .await
    }

    async fn complete_with(
        &self,
        model: &str,
        messages: &[Message],
        tools: &[ToolSpec],
        opts: &CompletionOptions,
        on_event: &mut EventSink<'_>,
    ) -> Result<ModelResponse, ProviderError> {
        self.refuse(model)?;
        let opts = CompletionOptions {
            deny_data_collection: true,
            ..opts.clone()
        };
        self.inner
            .complete_with(model, messages, tools, &opts, on_event)
            .await
    }

    fn stream_idle_hint(&self, model: &str) -> Option<std::time::Duration> {
        self.inner.stream_idle_hint(model)
    }

    fn context_fill(&self, model: &str, owner: &str) -> Option<u64> {
        self.inner.context_fill(model, owner)
    }
}
