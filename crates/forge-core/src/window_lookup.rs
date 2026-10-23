//! A pinned model's context-window lookup that runs beside startup instead of in front of it.
//!
//! The windows live in the store, and the session reads them from there each time it sizes a
//! request, so a lookup that lands later is applied by the next request on its own. What must not
//! happen is a request sized from the conservative floor that the real window would have sized
//! differently: compaction at 25.6k tokens, a trimmed history (the floor compaction the lookup
//! exists to prevent). So the session waits for the lookup only when the window can change what
//! it is about to do. A first request is a system prompt, the tools and one prompt: under every
//! limit the floor sets, so it goes out at once and the lookup refines the requests after it. That
//! wait was 0.6-1.2 s of the first answer in a TUI on a fresh store, where the prompt is typed
//! before the lookup is back.

use tokio::sync::watch;

use crate::{PresenterEvent, Session};

/// Receiving end of a background window lookup: `None` until it settles, then the pinned models
/// whose window is still unknown.
#[derive(Clone)]
pub struct WindowLookup(watch::Receiver<Option<Vec<String>>>);

impl WindowLookup {
    pub fn channel() -> (watch::Sender<Option<Vec<String>>>, Self) {
        let (tx, rx) = watch::channel(None);
        (tx, Self(rx))
    }

    /// The lookup has an answer, or its task is gone and never will.
    fn landed(&self) -> bool {
        self.0.borrow().is_some() || self.0.has_changed().is_err()
    }

    async fn settled(mut self) -> Vec<String> {
        loop {
            if let Some(unknown) = self.0.borrow().clone() {
                return unknown;
            }
            if self.0.changed().await.is_err() {
                return Vec::new();
            }
        }
    }
}

impl Session {
    /// Attach the lookup started at session build.
    pub fn set_window_lookup(&mut self, lookup: Option<WindowLookup>) {
        self.window_lookup = lookup;
    }

    /// Before sizing anything for `model` (a request when `specs` is given, a compaction check
    /// otherwise): take the startup window lookup's answer if it has one, and wait for it only when
    /// the window it may bring could change the outcome. Says so once when a window stays unknown.
    pub(crate) async fn settle_window_lookup(
        &mut self,
        model: &str,
        specs: Option<&[forge_provider::ToolSpec]>,
    ) {
        let Some(lookup) = self.window_lookup.as_ref() else {
            return;
        };
        if !lookup.landed() && self.window_cannot_matter(model, specs) {
            crate::turn_trace::mark("window lookup: not waited for");
            return;
        }
        let Some(lookup) = self.window_lookup.take() else {
            return;
        };
        let unknown = lookup.settled().await;
        if unknown.is_empty() {
            return;
        }
        let notice = format!(
            "context window of {} is unknown — requests and auto-compaction are sized for {} \
             tokens; set `[mesh.context_windows]` to the real window",
            unknown.join(", "),
            forge_mesh::pricing::CONSERVATIVE_CONTEXT_WINDOW
        );
        // A pin startup already called unknown does not also need its window explained: that is a
        // second warning about a model the person is about to be sent to /model over.
        let already_flagged = unknown.iter().all(|id| self.unverified_models.contains(id));
        if self.presenter.is_attended() && !already_flagged {
            self.presenter.emit(PresenterEvent::Warning(notice));
        } else {
            tracing::warn!("{notice}");
        }
    }

    /// Whether the window assumed for `model` now (the floor, while the lookup runs) already leaves
    /// this context alone: under a quarter of the compaction trigger, so neither compaction nor the
    /// elision floor moves, and, for a request, inside the trim budget beside the preamble and the
    /// tools. A larger window, all a lookup can bring, would then send exactly the same request.
    fn window_cannot_matter(
        &self,
        model: &str,
        specs: Option<&[forge_provider::ToolSpec]>,
    ) -> bool {
        let pressure = self.context_pressure_tokens();
        if pressure.saturating_mul(4) > self.compaction_trigger(model)
            || !self.transcript_fits(model)
        {
            return false;
        }
        let Some(specs) = specs else {
            return true;
        };
        let window = self.effective_context_window(model) as usize;
        let reserve = crate::output_planning_reserve_tokens(self.config.mesh.max_output_tokens);
        let fixed = self
            .system_preamble()
            .iter()
            .chain(self.pinned_context_message(Some(model)).iter())
            .map(crate::context_pipeline::message_tokens)
            .sum::<usize>()
            + crate::context_pipeline::tool_spec_tokens(specs)
            + if crate::codemode::guidance_advertised(specs) {
                crate::tokens::count_text(crate::codemode::GUIDANCE)
            } else {
                0
            };
        let budget = window
            .saturating_sub(reserve as usize)
            .saturating_sub(fixed)
            * 95
            / 100;
        self.estimated_transcript_tokens() as usize <= budget
    }
}
