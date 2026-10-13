//! `/btw <question>` (alias `/side`) — an inline side question that never enters the session
//! transcript (docs/features/side-questions.md).
//!
//! A side call in the `auxiliary_policy.rs` family: one model call whose answer is shown as a
//! `PresenterEvent::BtwAnswer` card and never pushed to the transcript, the message table, or a
//! usage row. Unlike the other side calls it can run WHILE A TURN IS BUSY: everything it needs
//! lives in a [`BtwAsker`] built from the session's shared parts (provider, router, store, config,
//! pin), refreshed at every turn start and reachable by session id through [`busy_asker`], so the
//! TUI never has to wait for the session lock the running turn holds. The question sees a bounded
//! excerpt of the conversation read from the store, which already holds the running turn's steps.
//!
//! Deliberately stateless: each `/btw` call is independent. Forge does not keep a running
//! side-conversation the way prime-agent's `/btw` does — see docs/features/side-questions.md for
//! why that's an intentional simplification, not an oversight. Tests live alongside the other
//! `Session::start`-based tests in `lib.rs`'s `mod tests` (same convention as `compact`).

use super::*;
use crate::compaction_policy::record_model_failure_in;

const BTW_SYSTEM: &str = "You are answering a quick side question asked out-of-band via /btw \
while a coding session continues. Below the question's context is an excerpt of that session's \
conversation (oldest first, long tool output shortened). You have no tools: answer from the \
excerpt and your own knowledge, directly and concisely, and say so plainly when the excerpt does \
not contain what is asked. This exchange is NOT part of the main conversation and will not be \
remembered.";

/// Characters of conversation the side question may see. Big enough for the recent work, small
/// enough that a side call on a 1M-token session stays cheap.
const CONTEXT_CHARS: usize = 60_000;
/// A single tool result or message beyond this is shortened, so one huge file read cannot crowd
/// out the rest of the excerpt.
const ITEM_CHARS: usize = 2_000;

/// Live askers by session id. Weak, so a finished session's asker is dropped with it.
static ASKERS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, std::sync::Weak<BtwAsker>>>,
> = std::sync::LazyLock::new(Default::default);

/// The asker for `session_id`, for a `/btw` issued while a turn holds the session lock. `None`
/// when that session has not started a turn in this process yet.
pub fn busy_asker(session_id: &str) -> Option<Arc<BtwAsker>> {
    ASKERS.lock().ok()?.get(session_id)?.upgrade()
}

/// Everything a `/btw` answer needs, detached from the `Session` so it runs beside a busy turn.
pub struct BtwAsker {
    session_id: String,
    provider: Arc<dyn Provider>,
    router: Arc<dyn Router>,
    store: Arc<Store>,
    config: Config,
    project: forge_types::ProjectContext,
    pinned_model: Option<Vec<String>>,
    pinned_effort: Option<EffortLevel>,
    catalog: Option<ModelCatalog>,
    /// Where a busy-path answer is shown; the idle path emits through the session's presenter.
    sink: Option<std::sync::Mutex<Box<dyn Presenter>>>,
}

impl BtwAsker {
    /// Answer `question` and show the card (or a warning) through this asker's presenter sink.
    pub async fn ask_and_emit(&self, question: &str) {
        let event = self.answer(question).await;
        if let Some(sink) = &self.sink {
            if let Ok(mut presenter) = sink.lock() {
                presenter.emit(event);
            }
        }
    }

    /// One best-effort side call. Never fails the caller: budget exhaustion, no available model,
    /// or a provider error come back as a `PresenterEvent::Warning` instead of a `BtwAnswer`.
    pub async fn answer(&self, question: &str) -> PresenterEvent {
        let question = question.trim();
        if question.is_empty() {
            return PresenterEvent::Warning("usage: /btw <question>".to_string());
        }
        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return PresenterEvent::Warning("/btw skipped — budget exhausted".to_string());
        }
        let readiness = readiness::ProviderReadiness::snapshot(&self.config, &self.store);
        let decision = self
            .router
            .route_hinted(
                question,
                false,
                budget,
                &readiness.health,
                &readiness.quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;
        let model = side_call_model(
            &self.config,
            &readiness.health,
            self.pinned_model.as_ref(),
            self.catalog.as_ref(),
            &decision,
        );
        if forge_provider::is_cli_bridge(&model) {
            return PresenterEvent::Warning(
                "/btw skipped — no non-bridge model available".to_string(),
            );
        }
        let mut messages = vec![Message::system(BTW_SYSTEM)];
        let excerpt = conversation_excerpt(&self.store, &self.session_id);
        if !excerpt.is_empty() {
            messages.push(Message::system(format!(
                "Session conversation excerpt:\n{excerpt}"
            )));
        }
        messages.push(Message::user(question.to_string()));
        let mut on_event = |_: StreamEvent| {};
        let completion_opts = Session::auxiliary_completion_options(&self.session_id, "btw");
        match self
            .provider
            .complete_with(&model, &messages, &[], &completion_opts, &mut on_event)
            .await
        {
            Ok(r) => PresenterEvent::BtwAnswer {
                question: question.to_string(),
                answer: r.content,
                model,
                cost_usd: Some(r.usage.cost_usd),
            },
            Err(e) => {
                record_model_failure_in(
                    &self.store,
                    &model,
                    &e,
                    std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs),
                );
                PresenterEvent::Warning(format!("/btw failed: {e}"))
            }
        }
    }
}

/// The most recent conversation, oldest first, within [`CONTEXT_CHARS`]: user and assistant text,
/// the tools the assistant called, and shortened tool results. A compaction summary, when the
/// store holds one, leads the excerpt so a long session's earlier work is not invisible.
fn conversation_excerpt(store: &Store, session_id: &str) -> String {
    let Ok(messages) = store.load_messages(session_id) else {
        return String::new();
    };
    let shorten = |text: &str| -> String {
        let text = text.trim();
        if text.chars().count() <= ITEM_CHARS {
            return text.to_string();
        }
        let head: String = text.chars().take(ITEM_CHARS).collect();
        format!("{head} …[shortened]")
    };
    let render = |m: &forge_store::StoredMessage| -> Option<String> {
        if m.visibility == forge_types::Visibility::UiOnly {
            return None;
        }
        match m.role {
            Role::User if !m.content.trim().is_empty() => {
                Some(format!("USER: {}", shorten(&m.content)))
            }
            Role::Assistant => {
                let mut line = String::new();
                if !m.content.trim().is_empty() {
                    line.push_str(&format!("ASSISTANT: {}", shorten(&m.content)));
                }
                if !m.tool_calls.is_empty() {
                    let names: Vec<&str> = m.tool_calls.iter().map(|c| c.name.as_str()).collect();
                    if !line.is_empty() {
                        line.push('\n');
                    }
                    line.push_str(&format!("ASSISTANT called: {}", names.join(", ")));
                }
                (!line.is_empty()).then_some(line)
            }
            Role::Tool if !m.content.trim().is_empty() => {
                Some(format!("TOOL RESULT: {}", shorten(&m.content)))
            }
            Role::System if m.content.starts_with("[Earlier conversation summarized") => {
                Some(shorten(&m.content))
            }
            _ => None,
        }
    };
    let mut picked = Vec::new();
    let mut used = 0usize;
    for message in messages.iter().rev() {
        let Some(text) = render(message) else {
            continue;
        };
        if used + text.len() > CONTEXT_CHARS && !picked.is_empty() {
            break;
        }
        used += text.len();
        picked.push(text);
    }
    picked.reverse();
    picked.join("\n\n")
}

/// The model for a cheap side call. A pinned session — the router's `--model` pin or the
/// in-session `/model` pin, which the router does not see — prefers the first usable trivial model
/// that is not a subscription, then its own pin; a routed session uses what routing chose. A
/// subscription model the session is not itself running on is never picked here: side calls are
/// optional and must not spend a plan the user did not point this session at.
pub(crate) fn side_call_model(
    config: &Config,
    health: &forge_types::ModelHealth,
    pinned_model: Option<&Vec<String>>,
    catalog: Option<&ModelCatalog>,
    routed: &forge_mesh::RoutingDecision,
) -> String {
    let session_pin = pinned_model.and_then(|set| set.first().cloned());
    if !(routed.pinned || session_pin.is_some()) {
        return routed.model.clone();
    }
    let own = session_pin.unwrap_or_else(|| routed.model.clone());
    // A side call that already failed permanently (model_not_found, auth, 402) is benched in the
    // shared store; without this filter the same doomed call repeated every turn.
    config
        .candidates_for(TaskTier::Trivial)
        .into_iter()
        .filter(|m| !forge_mesh::catalog::is_subscription(m) || *m == own)
        .filter(|m| crate::auxiliary_candidates::usable_for_side_call(m, health, catalog))
        .find(|m| forge_config::has_api_key(forge_config::provider_of(m)))
        .unwrap_or(own)
}

impl Session {
    /// A fresh [`BtwAsker`] for this session, registered so a `/btw` issued while a turn is busy
    /// finds it through [`busy_asker`]. Called at every turn start, so pins and config changed
    /// between turns are picked up.
    pub(crate) fn refresh_btw_asker(&mut self) -> Arc<BtwAsker> {
        let asker = Arc::new(BtwAsker {
            session_id: self.id.clone(),
            provider: Arc::clone(&self.provider),
            router: Arc::clone(&self.router),
            store: Arc::clone(&self.store),
            config: self.config.clone(),
            project: self.project.clone(),
            pinned_model: self.pinned_model.clone(),
            pinned_effort: self.pinned_effort,
            catalog: self.catalog.clone(),
            sink: self.presenter.recap_sink().map(std::sync::Mutex::new),
        });
        if let Ok(mut askers) = ASKERS.lock() {
            askers.retain(|_, weak| weak.strong_count() > 0);
            askers.insert(self.id.clone(), Arc::downgrade(&asker));
        }
        self.btw_asker = Some(Arc::clone(&asker));
        asker
    }

    /// Answer a `/btw`/`/side` question while the session is idle, emitting through the
    /// session's own presenter.
    pub async fn ask_btw(&mut self, question: &str) {
        let asker = self.refresh_btw_asker();
        let event = asker.answer(question).await;
        self.presenter.emit(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with(store: &Arc<Store>) -> Session {
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        Session::start(
            Arc::clone(store),
            Arc::new(forge_provider::MockProvider),
            Arc::new(HeuristicRouter::new(Config::default())),
            ToolRegistry::with_core_tools_in(workspace),
            Box::new(forge_tui::HeadlessPresenter::new(false)),
            Config::default(),
            workspace.to_str().unwrap(),
        )
        .unwrap()
    }

    /// The point of the busy path: a running turn holds the session lock for its whole length,
    /// so `/btw` used to queue behind it. The registered asker answers without that lock.
    #[tokio::test]
    async fn a_side_question_is_answered_while_the_session_lock_is_held() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = session_with(&store);
        session.run_turn("say hello").await.unwrap();
        let id = session.session_id().to_string();
        let shared = Arc::new(tokio::sync::Mutex::new(session));
        let _turn = shared.lock().await;

        let asker = busy_asker(&id).expect("a turn start registers the asker");
        match asker.answer("what is happening?").await {
            PresenterEvent::BtwAnswer { answer, .. } => assert!(!answer.is_empty()),
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn the_asker_goes_away_with_its_session() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = session_with(&store);
        session.run_turn("say hello").await.unwrap();
        let id = session.session_id().to_string();
        drop(session);
        assert!(busy_asker(&id).is_none());
    }

    #[tokio::test]
    async fn the_excerpt_carries_the_conversation_newest_last() {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let mut session = session_with(&store);
        session.run_turn("first-marker request").await.unwrap();
        session.run_turn("second-marker request").await.unwrap();
        let excerpt = conversation_excerpt(&store, session.session_id());
        let first = excerpt.find("first-marker").expect("first prompt included");
        let second = excerpt
            .find("second-marker")
            .expect("second prompt included");
        assert!(first < second, "oldest first: {excerpt}");
    }
}
