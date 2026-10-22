//! The best-effort calls a finished turn makes: a one-line recap, a predicted next prompt and the
//! extraction of durable memories.
//!
//! All three run on one trivial-tier side-call model. Choosing it reads the spend ledger, the
//! benched/quota snapshot and the catalog ranking, and on a real catalog that cost about a second
//! per call. On an interactive surface the answer and `Done` were already on screen, but the turn
//! task (the spinner, and the prompt the user queued next) waited for three such choices: the
//! "Finishing" phase that followed every answer. [`Session::spawn_post_turn_calls`] chooses once,
//! on a detached task, and runs the calls there. Headless callers keep running them inline so the
//! process does not exit before their results are stored.

use super::*;
use crate::auxiliary_policy::report_auxiliary_persistence_failure;
use crate::compaction_policy::record_model_failure_in;
use crate::spend_guard::priced;

/// The routing prompts each call is classified under (they only steer the tier hints).
const RECAP_ROUTE: &str = "summarize in one sentence";
const SUGGEST_ROUTE: &str = "propose the next likely user prompt";
const MEMORY_ROUTE: &str = "extract durable facts";

/// What choosing the side-call model needs, owned so the choice can run off the turn task.
pub(crate) struct SideModelInputs {
    config: Config,
    store: Arc<Store>,
    router: Arc<dyn Router>,
    pinned_model: Option<Vec<String>>,
    catalog: Option<ModelCatalog>,
    pinned_effort: Option<EffortLevel>,
    project: ProjectContext,
}

impl SideModelInputs {
    /// The model a best-effort side call routed under `route_prompt` uses, or `None` when the
    /// spend budget is exhausted or the only choice is a CLI bridge, which starts a whole external
    /// agent and is too heavy for an optional call.
    pub(crate) async fn decide(&self, route_prompt: &str) -> Option<String> {
        let (today, week, month) = self.store.spend_summary_usd().unwrap_or_default();
        let budget = BudgetState {
            spent_today_usd: today,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: week,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: month,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        if budget.status() == BudgetStatus::Exhausted {
            return None;
        }
        let readiness = readiness::ProviderReadiness::snapshot(&self.config, &self.store);
        // Under a single-model pin the side-call choice does not depend on routing (see
        // `side_call_model`), and ranking a real catalog is the expensive part.
        let pin = self
            .pinned_model
            .clone()
            .or_else(|| self.router.pin().map(<[String]>::to_vec));
        let decision = match pin.as_deref() {
            Some([only]) => forge_mesh::RoutingDecision {
                tier: TaskTier::Trivial,
                model: only.clone(),
                rationale: "pinned".to_string(),
                fallbacks: Vec::new(),
                pinned: true,
                unroutable: false,
                effort: None,
            },
            _ => {
                self.router
                    .route_hinted(
                        route_prompt,
                        false,
                        budget,
                        &readiness.health,
                        &readiness.quota,
                        Some(TaskTier::Trivial),
                        self.pinned_effort,
                        &self.project,
                    )
                    .await
            }
        };
        // Same rules as `Session::post_turn_auxiliary_model`: a pinned run keeps its own model
        // and skips the call when that is a subscription, otherwise a bridge is never used.
        if let Some(own) = crate::btw_policy::strict_pin_side_call_model(
            &self.config,
            self.pinned_model.as_ref(),
            &decision,
        ) {
            return (!forge_mesh::catalog::is_subscription(&own)).then_some(own);
        }
        let model = crate::btw_policy::side_call_model(
            &self.config,
            &readiness.health,
            self.pinned_model.as_ref(),
            self.catalog.as_ref(),
            &decision,
        );
        (!forge_provider::is_cli_bridge(&model)).then_some(model)
    }
}

/// What a side call produced for the user: events to show and persistence warnings.
type Emitted = Vec<PresenterEvent>;

/// One side-call completion, with everything it needs owned.
pub(crate) struct SideCall {
    provider: Arc<dyn Provider>,
    store: Arc<Store>,
    pricing: Pricing,
    id: String,
    cooldown: std::time::Duration,
    purpose: &'static str,
    messages: Vec<Message>,
}

impl SideCall {
    /// Run the completion on `model` and record its usage. `None` on a model error (the model is
    /// benched like any failed side call).
    async fn complete(&self, model: &str, out: &mut Emitted) -> Option<String> {
        let mut on_event = |_: StreamEvent| {};
        let options = Session::auxiliary_completion_options(&self.id, self.purpose);
        let response = match self
            .provider
            .complete_with(model, &self.messages, &[], &options, &mut on_event)
            .await
        {
            Ok(response) => response,
            Err(error) => {
                record_model_failure_in(&self.store, model, &error, self.cooldown);
                return None;
            }
        };
        if let Err(error) = self.store.record_side_call_usage_for(
            &self.id,
            self.purpose,
            Some(model),
            &priced(&self.pricing, model, &response.usage),
        ) {
            let label = match self.purpose {
                "suggest" => "suggestion usage".to_string(),
                purpose => format!("{purpose} usage"),
            };
            report_auxiliary_persistence_failure(&self.id, &label, &error, |warning| {
                out.push(PresenterEvent::Warning(warning))
            });
        }
        Some(response.content)
    }
}

/// The recap call; its reply becomes one [`PresenterEvent::Recap`] row.
pub(crate) struct RecapCall(SideCall);

impl RecapCall {
    /// Bounded: headless exit waits on it, and a slow model must not hold the process open.
    async fn run(self, model: &str, bounded: bool) -> Emitted {
        let mut out = Vec::new();
        let call = self.0.complete(model, &mut out);
        let content = if bounded {
            match tokio::time::timeout(std::time::Duration::from_secs(20), call).await {
                Ok(content) => content,
                Err(_) => return out,
            }
        } else {
            call.await
        };
        if let Some(text) = content.as_deref().and_then(recap_line) {
            out.push(PresenterEvent::Recap { text });
        }
        out
    }
}

/// The next-prompt suggestion; shown as ghost text in the idle input box.
pub(crate) struct SuggestCall {
    call: SideCall,
    prev_prompt: String,
}

impl SuggestCall {
    async fn run(self, model: &str) -> Emitted {
        let mut out = Vec::new();
        if let Some(content) = self.call.complete(model, &mut out).await {
            if let Some(text) = sanitize_suggestion(&content, &self.prev_prompt) {
                out.push(PresenterEvent::SuggestionReady { text });
            }
        }
        out
    }
}

/// Memory extraction: up to three durable facts, stored as project-scoped memories.
pub(crate) struct MemoryCall {
    call: SideCall,
    config: Config,
    workspace: WorkspaceContext,
}

impl MemoryCall {
    async fn run(self, model: &str) -> Emitted {
        let mut out = Vec::new();
        let Some(content) = self.call.complete(model, &mut out).await else {
            return out;
        };
        let store = &self.call.store;
        let id = &self.call.id;
        let scope = memory_scope_at(self.workspace.root());
        // The extractor quotes whatever the turn contained, and a stored memory is injected into
        // every later prompt of the project: mask credentials before they persist.
        let redactor = tokio::task::spawn_blocking(crate::redact::Redactor::from_environment)
            .await
            .unwrap_or_default();
        let lines: Vec<String> = content
            .lines()
            .take(3)
            .map(|line| redactor.redact(line).into_owned())
            .collect();
        for raw in lines {
            let line = raw.trim().trim_start_matches(['-', '*', '•']).trim();
            let Some((kind, text)) = line.split_once(':') else {
                continue;
            };
            let kind_norm = kind.trim().to_lowercase();
            let kind_cat = match kind_norm.as_str() {
                "preference" | "decision" | "fact" | "reference" => kind_norm.as_str(),
                _ => "fact",
            };
            let text = text.trim();
            if text.len() < 4 {
                continue;
            }
            let stored = match embed_one(&self.config.lattice.embeddings, text).await {
                Some(emb) => store.add_memory_with_embedding(&scope, kind_cat, text, id, &emb),
                None => store.add_memory(&scope, kind_cat, text, id),
            };
            if let Err(error) = stored {
                report_auxiliary_persistence_failure(id, "memory", &error, |warning| {
                    out.push(PresenterEvent::Warning(warning))
                });
            }
        }
        crate::memory_scope::align_memory_scopes(store, self.workspace.root());
        out
    }
}

impl Session {
    const RECAP_SYSTEM: &'static str = "You are a one-line summarizer for a coding assistant. \
Given the user's request and the assistant's response, write a SINGLE sentence (≤12 words, \
past tense, no punctuation at end) describing ONLY what the assistant's RESPONSE actually shows it \
did — never assume the request was fulfilled. If the response does not clearly show completed \
work (it stalled, errored, only planned, or asked a question), say that instead (e.g. \
\"stalled without completing the task\"). Do not invent success. \
Output ONLY that sentence — no preamble, no quotation marks.";

    const SUGGEST_SYSTEM: &'static str = "You are predicting a coding assistant user's NEXT \
prompt, Claude-Code-style. Given the user's last request and the tail of the assistant's \
response, propose the SINGLE most likely next thing this user will ask for: a short imperative \
instruction, ≤120 characters, no quotation marks, no markdown, no preamble. Output ONLY the \
prompt text, nothing else.";

    pub(crate) const MEMORY_CAPTURE_SYSTEM: &'static str =
        "You extract DURABLE facts worth remembering across FUTURE sessions in this project: user \
         preferences, project decisions/conventions, key architecture or config, and stable \
         constraints. Output 0 to 3 lines, each exactly `kind: fact`, where kind is one of \
         preference, decision, fact, reference. Skip transient task details, one-off actions, and \
         anything specific to only this turn. If nothing is durable, output NOTHING at all.";

    pub(crate) fn side_model_inputs(&self) -> SideModelInputs {
        SideModelInputs {
            config: self.config.clone(),
            store: self.store.clone(),
            router: self.router.clone(),
            pinned_model: self.pinned_model.clone(),
            catalog: self.catalog.clone(),
            pinned_effort: self.pinned_effort,
            project: self.project.clone(),
        }
    }

    fn side_call(&self, purpose: &'static str, messages: Vec<Message>) -> SideCall {
        SideCall {
            provider: self.provider.clone(),
            store: self.store.clone(),
            pricing: self.pricing.clone(),
            id: self.id.clone(),
            cooldown: std::time::Duration::from_secs(self.config.mesh.failover_cooldown_secs),
            purpose,
            messages,
        }
    }

    /// The recap call for this turn, or `None` when no model call is needed: recaps are off, the
    /// surface does not show them, the turn produced nothing to summarize, or the finished task
    /// list already says what was done (that recap is emitted here, without a model).
    fn recap_call(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) -> Option<RecapCall> {
        if !self.config.recap.enabled || !self.presenter.consumes_recap() {
            return None;
        }
        // A stalled turn leaves `final_text` empty: recapping it makes the summarizer lean on the
        // *request* and invent success for a turn that accomplished nothing.
        if final_text.trim().is_empty() {
            return None;
        }
        if let Some(text) = completed_tasks_recap(tasks_before, &self.tasks, final_text) {
            self.presenter.emit(PresenterEvent::Recap { text });
            return None;
        }
        let user_snippet: String = prompt.chars().take(1000).collect();
        // A long answer ends with its conclusion, which is what a recap is about: keep the tail.
        let assistant_snippet: String = if final_text.chars().count() > 2000 {
            let head: String = final_text.chars().take(600).collect();
            let tail: Vec<char> = final_text.chars().rev().take(1400).collect();
            format!("{head} … {}", tail.into_iter().rev().collect::<String>())
        } else {
            final_text.to_string()
        };
        let messages = vec![
            Message::system(Self::RECAP_SYSTEM),
            Message::user(format!(
                "User request:\n{user_snippet}\n\nAssistant response:\n{assistant_snippet}"
            )),
        ];
        Some(RecapCall(self.side_call("recap", messages)))
    }

    fn suggest_call(&self, prompt: &str, final_text: &str) -> Option<SuggestCall> {
        if !self.config.suggest.enabled
            || !self.presenter.consumes_suggestions()
            || final_text.trim().is_empty()
        {
            return None;
        }
        let user_snippet: String = prompt.chars().take(400).collect();
        // The TAIL of the response is what best predicts a follow-up: it is what the user is
        // looking at right now.
        let assistant_chars: Vec<char> = final_text.chars().collect();
        let tail_start = assistant_chars.len().saturating_sub(2000);
        let assistant_snippet: String = assistant_chars[tail_start..].iter().collect();
        let messages = vec![
            Message::system(Self::SUGGEST_SYSTEM),
            Message::user(format!(
                "User's last prompt:\n{user_snippet}\n\nAssistant response (tail):\n{assistant_snippet}"
            )),
        ];
        Some(SuggestCall {
            call: self.side_call("suggest", messages),
            prev_prompt: prompt.to_string(),
        })
    }

    fn memory_call(&self, prompt: &str, final_text: &str) -> Option<MemoryCall> {
        if !self.config.mesh.auto_memory || final_text.trim().is_empty() {
            return None;
        }
        let user_snippet: String = prompt.chars().take(500).collect();
        let assistant_snippet: String = final_text.chars().take(1200).collect();
        let messages = vec![
            Message::system(Session::MEMORY_CAPTURE_SYSTEM),
            Message::user(format!(
                "User request:\n{user_snippet}\n\nAssistant response:\n{assistant_snippet}"
            )),
        ];
        Some(MemoryCall {
            call: self.side_call("memory", messages),
            config: self.config.clone(),
            workspace: self.workspace.clone(),
        })
    }

    /// Interactive surfaces only: run the turn's recap, suggestion and memory capture on one
    /// detached task that chooses their model once. Returns at once; results arrive through the
    /// presenter's sink.
    pub(crate) fn spawn_post_turn_calls(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) {
        let recap = self.recap_call(prompt, final_text, tasks_before);
        let suggest = self.suggest_call(prompt, final_text);
        let memory = self.memory_call(prompt, final_text);
        if recap.is_none() && suggest.is_none() && memory.is_none() {
            return;
        }
        let Some(mut sink) = self.presenter.recap_sink() else {
            return;
        };
        let inputs = self.side_model_inputs();
        tokio::spawn(async move {
            let Some(model) = inputs.decide(RECAP_ROUTE).await else {
                return;
            };
            let (recap_out, suggest_out) = futures::join!(
                async {
                    match recap {
                        Some(call) => call.run(&model, false).await,
                        None => Vec::new(),
                    }
                },
                async {
                    match suggest {
                        Some(call) => call.run(&model).await,
                        None => Vec::new(),
                    }
                }
            );
            for event in recap_out.into_iter().chain(suggest_out) {
                sink.emit(event);
            }
            if let Some(call) = memory {
                for event in call.run(&model).await {
                    sink.emit(event);
                }
            }
        });
    }

    /// After a turn completes, make one cheap trivial-tier call to generate a one-line recap,
    /// emitted via [`PresenterEvent::Recap`]. Best-effort: silently skipped on budget exhaustion
    /// or any model error so it can never derail the session. A channel-backed presenter gets the
    /// whole call (model choice included) on a detached task; others run it inline.
    pub(crate) async fn generate_recap(
        &mut self,
        prompt: &str,
        final_text: &str,
        tasks_before: &[forge_types::TodoItem],
    ) {
        let Some(call) = self.recap_call(prompt, final_text, tasks_before) else {
            return;
        };
        let inputs = self.side_model_inputs();
        match self.presenter.recap_sink() {
            Some(mut sink) => {
                tokio::spawn(async move {
                    if let Some(model) = inputs.decide(RECAP_ROUTE).await {
                        for event in call.run(&model, false).await {
                            sink.emit(event);
                        }
                    }
                });
            }
            None => {
                let Some(model) = inputs.decide(RECAP_ROUTE).await else {
                    return;
                };
                for event in call.run(&model, true).await {
                    self.presenter.emit(event);
                }
            }
        }
    }

    /// After a turn completes, make one cheap trivial-tier call predicting the user's likely
    /// next prompt, emitted via [`PresenterEvent::SuggestionReady`] and shown as dim ghost text
    /// in an empty, idle input box (Tab accepts it — editable, never auto-sent). Best-effort,
    /// detached on a channel-backed presenter exactly like [`Self::generate_recap`].
    pub(crate) async fn generate_suggestion(&mut self, prompt: &str, final_text: &str) {
        let Some(call) = self.suggest_call(prompt, final_text) else {
            return;
        };
        let inputs = self.side_model_inputs();
        match self.presenter.recap_sink() {
            Some(mut sink) => {
                tokio::spawn(async move {
                    if let Some(model) = inputs.decide(SUGGEST_ROUTE).await {
                        for event in call.run(&model).await {
                            sink.emit(event);
                        }
                    }
                });
            }
            None => {
                let Some(model) = inputs.decide(SUGGEST_ROUTE).await else {
                    return;
                };
                for event in call.run(&model).await {
                    self.presenter.emit(event);
                }
            }
        }
    }

    /// After a turn, make one cheap trivial-tier call to extract 0-3 DURABLE facts and persist them
    /// as project-scoped memories (dedup + salience handled by the store). Best-effort: any
    /// budget/model failure is silently skipped so it can never derail the session. Recall of these
    /// happens at the start of a later session (see `run_turn_with`).
    ///
    /// The model is chosen here, so a session pin goes through the same side-call filter as
    /// recap/suggestion. The returned handle lets one-shot mode (`forge run`) await persistence
    /// before the process exits; interactive turns drop it and it finishes in the background.
    pub(crate) async fn capture_memories(
        &mut self,
        prompt: &str,
        final_text: &str,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let call = self.memory_call(prompt, final_text)?;
        let model = self.side_model_inputs().decide(MEMORY_ROUTE).await?;
        let mut warning_sink = self.presenter.recap_sink();
        Some(tokio::spawn(async move {
            for event in call.run(&model).await {
                if let Some(sink) = warning_sink.as_mut() {
                    sink.emit(event);
                }
            }
        }))
    }
}
