//! Continual Harness (`/refine`, port of prime-agent's `/refine`): the agent proposes small,
//! evidence-backed durable edits to its own prompt/skill/subagent entries, journaled through
//! forge-store's `harness_entry`/`harness_refinement` tables (see `forge_store::harness_store`).
//!
//! The model call mirrors [`Session::compact`] (compaction_policy.rs): a trivial-tier candidate
//! chain that never hard-fails on an unreachable cheap model, and the same transcript-fitting
//! helpers so a long session's harness prompt still fits the summarizer's window.

use forge_store::{HarnessEdit, HarnessEntry, HarnessRefinement};

use super::*;

/// Harness entries with no explicit scope live here — shared across every project/session.
const GLOBAL_SCOPE: &str = "global";

fn session_scope(session_id: &str) -> String {
    format!("session:{session_id}")
}

fn project_scope(root: impl std::fmt::Display) -> String {
    format!("project:{root}")
}

/// Raw slice of the transcript handed to the refinement model: capped BEFORE token-fitting so
/// rendering it (and searching for a fit) stays cheap even in a very long session.
const REFINE_TRAJECTORY_MESSAGES: usize = 60;
/// How many past refinement batches for this session are shown as history, so the model doesn't
/// repeat or thrash a decision it (or a rollback) already made.
const REFINE_HISTORY_LIMIT: usize = 5;
/// Reply room reserved for the refinement JSON itself when sizing the request — mirrors
/// [`COMPACT_SUMMARY_RESERVE_TOKENS`], the same reasoning: the answer is small and fixed-shape,
/// so this does not need the main loop's large planning cushion.
const REFINE_RESPONSE_RESERVE_TOKENS: usize = 2_048;

const REFINE_SYSTEM: &str = "You are Forge's Continual Harness: after real work, you propose \
small, evidence-backed durable improvements to how FORGE ITSELF operates on future turns — never \
to the user's own codebase.

You maintain three kinds of harness entries:
- prompt: a supplemental prompt note, added ALONGSIDE Forge's base system prompt. That base \
prompt is immutable and must never be restated or rewritten here — only propose ADDITIONS: a \
convention, a pitfall to avoid, a house-style rule.
- skill: a reusable procedure — when to use it, the concrete steps, how to verify it worked. \
Reference Forge's own skills/commands/tools by name; never propose Python or any other runtime.
- subagent: a reusable delegation spec for Forge's spawn_agents tool — its purpose, the \
instructions to hand it, and when a future turn should invoke it.

Do NOT propose `memory` entries, or anything that is really a durable fact/preference/decision — \
Forge's separate auto-memory system already owns those; a memory-shaped edit here is rejected.

You will be given: a bounded slice of the recent conversation, the harness entries currently in \
scope (id, scope, kind, title, content — entries from a scope OTHER than the target scope are \
READ-ONLY reference context, never propose editing or deleting them), and the last few \
refinement batches already applied to this session.

Only propose edits you can justify from concrete evidence in the conversation above — a mistake \
made and corrected, a repeated pattern, an instruction that had to be given twice. Prefer a SMALL \
number of precise edits over a sweeping rewrite. An `update`/`delete` MUST name an `id` that \
appears in the harness entries given to you and that belongs to the target scope; never invent \
one. A `create` never sets `id`. If nothing is worth changing, propose no edits at all.

Output ONLY this JSON object, nothing else, no markdown fences:
{\"summary\": \"one line: what changed and why\", \"rationale\": \"why these edits improve future \
turns\", \"expectedOutcome\": \"what should be different next time\", \"edits\": [{\"action\": \
\"create|update|delete\", \"kind\": \"prompt|skill|subagent\", \"id\": \"<required for \
update/delete>\", \"title\": \"<for create/update>\", \"content\": \"<for create/update>\", \
\"reason\": \"<one line>\"}]}";

impl Session {
    /// Propose and apply one batch of harness edits from the current session's trajectory. The
    /// target scope is `session:<id>` unless `global` is requested; entries from the other scopes
    /// (still read by the model as context) are never written by this call — the store itself
    /// enforces that per-scope boundary (`apply_harness_edits` checks the target scope on every
    /// update/delete). `trigger` is journaled verbatim: `"manual"` | `"auto_turns"` |
    /// `"auto_compact"`.
    pub async fn refine(
        &mut self,
        instructions: Option<&str>,
        global: bool,
        trigger: &str,
    ) -> Result<HarnessRefinement, CoreError> {
        if !self.config.harness.enabled {
            return Err(CoreError::Internal(
                "Continual Harness is disabled (harness.enabled = false)".to_string(),
            ));
        }
        // Any invocation — manual or auto — restarts the turns-since-last-refine clock, so an
        // auto gate never immediately re-fires on the very next eligible turn.
        self.turns_since_refine = 0;

        let target_scope = if global {
            GLOBAL_SCOPE.to_string()
        } else {
            session_scope(&self.id)
        };

        let trajectory: Vec<Message> = {
            let mut recent: Vec<Message> = self
                .transcript
                .iter()
                .rev()
                .filter(|m| m.visibility.is_llm())
                .take(REFINE_TRAJECTORY_MESSAGES)
                .cloned()
                .collect();
            recent.reverse();
            recent
        };
        let overview = self.harness_overview()?;
        let history = self
            .store
            .harness_refinements(Some(&self.id), REFINE_HISTORY_LIMIT)?;
        let entries = refine_payload_entries(&trajectory, &overview, &target_scope, &history);

        let budget = BudgetState {
            spent_today_usd: self.store.spend_today_usd()?,
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: self.store.spend_this_week_usd()?,
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: self.store.spend_this_month_usd()?,
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        let readiness = self.provider_readiness();
        let health = readiness.health;
        let quota = readiness.quota;
        let decision = self
            .router
            .route_hinted(
                "refine the harness based on this session",
                false,
                budget,
                &health,
                &quota,
                Some(TaskTier::Trivial),
                self.pinned_effort,
                &self.project,
            )
            .await;

        // Same never-hard-fail-on-a-cheap-model chain compaction uses (see `Session::compact`):
        // top trivial candidates, then the routed model + its own fallbacks, then the session's
        // own guaranteed-reachable model as a backstop.
        let failover = self.config.mesh.failover;
        let guaranteed = self
            .pinned_model()
            .and_then(|set| set.first().cloned())
            .unwrap_or_else(|| decision.model.clone());
        let mut routed = vec![self.auxiliary_model(&decision)];
        routed.extend(decision.fallbacks.clone());
        let candidates =
            compact_candidate_chain(self.router.trivial_candidates(), routed, &guaranteed, |m| {
                health.is_benched(m)
            });
        let mut chain = candidates.into_iter();
        let mut model = chain.next().expect("compact_candidate_chain is non-empty");
        let completion_opts = Self::auxiliary_completion_options(&self.id, "refine");
        let instructions_owned = instructions.map(str::to_string);

        let resp = loop {
            let mut sink = |_: StreamEvent| {};
            // Re-fit to THIS candidate's window every hop, exactly like `compact` — the chain
            // deliberately crosses models with very different context sizes.
            let budget_tokens = self.refine_input_budget(&model);
            let payload = fit_compaction_payload(&entries, budget_tokens);
            let user = refine_user_message(instructions_owned.as_deref(), &target_scope, &payload);
            let messages = [Message::system(REFINE_SYSTEM), Message::user(user)];
            match self
                .provider
                .complete_with(&model, &messages, &[], &completion_opts, &mut sink)
                .await
            {
                Ok(r) => break r,
                Err(e) if failover => match self.advance_fallback(&model, &e, &mut chain, "refine")
                {
                    Some(next) => model = next,
                    None => return Err(CoreError::Provider(e)),
                },
                Err(e) => return Err(CoreError::Provider(e)),
            }
        };
        let _ = self
            .store
            .record_side_call_usage(&self.id, "refine", &resp.usage);

        let parsed = parse_refine_response(&resp.content).ok_or_else(|| {
            CoreError::Internal(format!(
                "refine: model response was not the expected JSON shape: {}",
                resp.content.chars().take(200).collect::<String>()
            ))
        })?;

        let max_chars = self.config.harness.max_entry_chars as usize;
        let validated: Vec<ValidatedEdit> = parsed
            .edits
            .into_iter()
            .map(|e| validate_refine_edit(e, max_chars))
            .collect();
        let to_apply: Vec<HarnessEdit> = validated
            .iter()
            .filter_map(|v| match v {
                ValidatedEdit::Accepted(e) => Some(e.clone()),
                ValidatedEdit::Rejected(_) => None,
            })
            .collect();

        let mut refinement = self.store.apply_harness_edits(
            &target_scope,
            &self.id,
            trigger,
            &parsed.summary,
            &parsed.rationale,
            &parsed.expected_outcome,
            to_apply,
        )?;
        refinement.edits = merge_validated_results(validated, refinement.edits);

        let applied = refinement.edits.iter().filter(|e| e.applied).count();
        let rejected = refinement.edits.len() - applied;
        self.presenter.emit(PresenterEvent::Warning(format!(
            "harness refinement ({trigger}): {} — {applied} edit(s) applied, {rejected} rejected \
             (via {model})",
            refinement.summary
        )));

        Ok(refinement)
    }

    /// Undo a past refinement batch: inverts every applied edit from its journaled before/after
    /// snapshot and journals the inversion as a fresh `trigger = "rollback"` refinement. Thin
    /// passthrough — [`forge_store::Store::rollback_harness_refinement`] does the real work.
    pub fn refine_rollback(&mut self, refinement_id: &str) -> Result<HarnessRefinement, CoreError> {
        self.store
            .rollback_harness_refinement(refinement_id, &self.id)
            .map_err(CoreError::from)
    }

    /// Harness entries visible to this session, in scope precedence order (session, then
    /// project, then global) — for the `/refine` status view and as the model-facing overview
    /// during a refinement pass. Unfiltered by `harness.enabled`: a status view should show what
    /// is stored even while injection/refinement is switched off.
    pub fn harness_overview(&self) -> Result<Vec<HarnessEntry>, CoreError> {
        let mut out = Vec::new();
        for scope in self.harness_scope_chain() {
            out.extend(self.store.harness_entries(&[scope.as_str()])?);
        }
        Ok(out)
    }

    /// The three scopes this session's harness context is drawn from, most-specific first:
    /// `session:<id>`, then `project:<workspace root>`, then `global`.
    fn harness_scope_chain(&self) -> [String; 3] {
        [
            session_scope(&self.id),
            project_scope(self.workspace.display()),
            GLOBAL_SCOPE.to_string(),
        ]
    }

    /// Token budget for one refinement request against `model`: its window, minus [`REFINE_SYSTEM`]
    /// and reply room, with the same 5% headroom `compact_input_budget` keeps for tokenizer
    /// divergence. Floored so even a tiny-window trivial model gets a request worth making.
    fn refine_input_budget(&self, model: &str) -> usize {
        let window = self.effective_context_window(model) as usize;
        let reserve = REFINE_RESPONSE_RESERVE_TOKENS + tokens::count_message(REFINE_SYSTEM);
        (window.saturating_sub(reserve) * 95 / 100).max(512)
    }

    /// Auto-refine gate for `harness.auto_refine = "turns"`: run a refinement pass every
    /// `auto_refine_turns` completed turns. Best-effort like the other post-turn side calls
    /// (recap/suggestion/memory) — never fails the turn. Deliberately has no separate LLM
    /// gate-review pre-call before the refinement itself; unlike prime-agent's `/refine`, Forge
    /// keeps this simple and treats the turn interval alone as the gate.
    pub(crate) async fn auto_refine_after_turns(&mut self) {
        if !self.config.harness.enabled
            || self.config.harness.auto_refine != forge_config::AutoRefineMode::Turns
        {
            return;
        }
        self.turns_since_refine += 1;
        if !should_auto_refine_turns(
            self.turns_since_refine,
            self.config.harness.auto_refine_turns,
        ) {
            return;
        }
        let _ = self.refine(None, false, "auto_turns").await;
    }
}

/// Pure gate for `harness.auto_refine = "turns"`: fire once `turns_since_refine` reaches the
/// configured interval. `auto_refine_turns == 0` never fires (treated as "disabled", not
/// "every turn") so a misconfigured zero can't hammer the model on every single turn.
fn should_auto_refine_turns(turns_since_refine: u32, auto_refine_turns: u32) -> bool {
    auto_refine_turns > 0 && turns_since_refine >= auto_refine_turns
}

/// Render one transcript message the same way `Session::compact`'s entries builder does: role,
/// content, then any tool calls the assistant made (the only record of what a turn actually did).
fn render_transcript_entry(m: &Message) -> String {
    let mut line = format!("{}: {}", m.role.as_str(), m.content);
    for tc in &m.tool_calls {
        line.push_str(&format!("\n  [call {} {}]", tc.name, tc.args));
    }
    line
}

fn render_harness_entry_line(e: &HarnessEntry) -> String {
    format!(
        "- id={} scope={} kind={} title={:?}\n  {}",
        e.id, e.scope, e.kind, e.title, e.content
    )
}

fn render_refinement_history_line(r: &HarnessRefinement) -> String {
    format!(
        "- [{}] trigger={} summary={:?} ({} edit(s), {} applied)",
        r.created_at,
        r.trigger,
        r.summary,
        r.edits.len(),
        r.edits.iter().filter(|e| e.applied).count()
    )
}

/// Build the ordered list of renderable "entries" `fit_compaction_payload` trims to a token
/// budget: the trajectory slice, then the in-scope harness overview, then recent refinement
/// history. One shared list (rather than three separately-budgeted blocks) so the SAME helper
/// compaction uses caps the WHOLE payload the way compaction caps its own input.
fn refine_payload_entries(
    trajectory: &[Message],
    overview: &[HarnessEntry],
    target_scope: &str,
    history: &[HarnessRefinement],
) -> Vec<String> {
    let mut entries: Vec<String> = trajectory.iter().map(render_transcript_entry).collect();
    entries.push(format!(
        "--- harness entries currently in scope (target scope for this refinement: {target_scope}; \
         entries from any other scope are READ-ONLY) ---"
    ));
    if overview.is_empty() {
        entries.push("(none yet)".to_string());
    } else {
        entries.extend(overview.iter().map(render_harness_entry_line));
    }
    entries.push("--- recent refinement history for this session ---".to_string());
    if history.is_empty() {
        entries.push("(none yet)".to_string());
    } else {
        entries.extend(history.iter().map(render_refinement_history_line));
    }
    entries
}

fn refine_user_message(instructions: Option<&str>, target_scope: &str, payload: &str) -> String {
    let mut out = String::new();
    if let Some(extra) = instructions {
        out.push_str(&format!(
            "User-requested focus for this refinement:\n{extra}\n\n"
        ));
    }
    out.push_str(&format!(
        "Target scope for new/updated entries: {target_scope}\n\n"
    ));
    out.push_str(payload);
    out
}

/// The strict JSON shape [`REFINE_SYSTEM`] demands. `edits` reuses [`HarnessEdit`]'s own
/// `Deserialize` impl directly — its field names already match the demanded JSON keys — so a
/// structurally malformed edit fails the whole parse (caught by the caller as a hard `Err`); only
/// semantically-invalid-but-well-formed edits (unknown `action`/`kind`) are handled softly by
/// [`validate_refine_edit`].
#[derive(Debug, serde::Deserialize)]
struct RefineResponse {
    summary: String,
    rationale: String,
    #[serde(rename = "expectedOutcome")]
    expected_outcome: String,
    #[serde(default)]
    edits: Vec<HarnessEdit>,
}

/// Parse a (possibly prose/fence-wrapped) model reply into a [`RefineResponse`], tolerant of
/// surrounding text the way `assay.rs`'s `parse_candidates`/`parse_verdict` are — pulls out the
/// substring between the first `{` and the last `}` before handing it to serde.
fn parse_refine_response(text: &str) -> Option<RefineResponse> {
    let json = slice_between(text, '{', '}')?;
    serde_json::from_str(json).ok()
}

fn slice_between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)?;
    let end = text.rfind(close)?;
    (end >= start).then(|| &text[start..=end])
}

/// A proposed edit after per-edit validation: either accepted (and, for `create`/`update`,
/// content-clamped) for the store to apply, or rejected with the reason recorded so the caller
/// can surface it — mirrors exactly what `apply_harness_edits` itself does for an unknown
/// action/id, just one layer earlier for checks the store can't make (kind is not validated
/// there; any string is accepted).
enum ValidatedEdit {
    Accepted(HarnessEdit),
    Rejected(Box<forge_store::AppliedHarnessEdit>),
}

const ALLOWED_HARNESS_KINDS: [&str; 3] = ["prompt", "skill", "subagent"];

fn validate_refine_edit(mut edit: HarnessEdit, max_content_chars: usize) -> ValidatedEdit {
    if !matches!(edit.action.as_str(), "create" | "update" | "delete") {
        let reason = format!("unknown edit action '{}'", edit.action);
        return ValidatedEdit::Rejected(reject_edit(edit, reason));
    }
    if !ALLOWED_HARNESS_KINDS.contains(&edit.kind.as_str()) {
        let reason = if edit.kind == "memory" {
            "memory-kind edits are not allowed here — Forge's auto-memory system owns durable \
             facts"
                .to_string()
        } else {
            format!("unsupported harness kind '{}'", edit.kind)
        };
        return ValidatedEdit::Rejected(reject_edit(edit, reason));
    }
    if let Some(content) = edit.content.as_mut() {
        clamp_in_place(content, max_content_chars);
    }
    ValidatedEdit::Accepted(edit)
}

fn clamp_in_place(content: &mut String, max_chars: usize) {
    if content.chars().count() <= max_chars {
        return;
    }
    let mut truncated: String = content.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    *content = truncated;
}

fn reject_edit(edit: HarnessEdit, error: String) -> Box<forge_store::AppliedHarnessEdit> {
    let id = edit.id.clone().unwrap_or_default();
    Box::new(forge_store::AppliedHarnessEdit {
        edit,
        id,
        before: None,
        after: None,
        applied: false,
        error: Some(error),
    })
}

/// Recombine the store's applied results with the pre-rejected edits, restoring the model's
/// original proposal order. `applied` must contain exactly one result per
/// [`ValidatedEdit::Accepted`] in `validated`, in the same relative order — true by construction,
/// since `apply_harness_edits` processes its input `Vec<HarnessEdit>` in order and
/// `to_apply` in [`Session::refine`] is built by filtering `validated` in order.
fn merge_validated_results(
    validated: Vec<ValidatedEdit>,
    applied: Vec<forge_store::AppliedHarnessEdit>,
) -> Vec<forge_store::AppliedHarnessEdit> {
    let mut applied = applied.into_iter();
    validated
        .into_iter()
        .map(|v| match v {
            ValidatedEdit::Accepted(_) => {
                applied.next().expect("one store result per accepted edit")
            }
            ValidatedEdit::Rejected(r) => *r,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(action: &str, kind: &str, content: Option<&str>) -> HarnessEdit {
        HarnessEdit {
            action: action.to_string(),
            kind: kind.to_string(),
            id: None,
            title: Some("t".to_string()),
            content: content.map(str::to_string),
            reason: Some("evidence".to_string()),
        }
    }

    fn applied(id: &str) -> forge_store::AppliedHarnessEdit {
        forge_store::AppliedHarnessEdit {
            edit: edit("create", "prompt", Some("c")),
            id: id.to_string(),
            before: None,
            after: None,
            applied: true,
            error: None,
        }
    }

    // --- parse_refine_response ---------------------------------------------------------------

    #[test]
    fn parse_refine_response_happy_path() {
        let raw = r#"Sure, here you go:
{"summary": "added a note", "rationale": "seen twice", "expectedOutcome": "fewer repeats",
 "edits": [{"action": "create", "kind": "prompt", "title": "t", "content": "c", "reason": "r"}]}
Hope that helps!"#;
        let parsed = parse_refine_response(raw).expect("valid JSON embedded in prose");
        assert_eq!(parsed.summary, "added a note");
        assert_eq!(parsed.expected_outcome, "fewer repeats");
        assert_eq!(parsed.edits.len(), 1);
        assert_eq!(parsed.edits[0].action, "create");
    }

    #[test]
    fn parse_refine_response_empty_edits_is_valid() {
        let raw = r#"{"summary": "s", "rationale": "r", "expectedOutcome": "e", "edits": []}"#;
        let parsed = parse_refine_response(raw).unwrap();
        assert!(parsed.edits.is_empty());
    }

    #[test]
    fn parse_refine_response_rejects_non_json() {
        assert!(parse_refine_response("I don't think any changes are needed.").is_none());
    }

    // --- validate_refine_edit --------------------------------------------------------------

    #[test]
    fn validate_refine_edit_accepts_known_kinds() {
        for kind in ALLOWED_HARNESS_KINDS {
            let v = validate_refine_edit(edit("create", kind, Some("c")), 2000);
            assert!(
                matches!(v, ValidatedEdit::Accepted(_)),
                "kind {kind} should be accepted"
            );
        }
    }

    #[test]
    fn validate_refine_edit_rejects_memory_kind_with_explanation() {
        let v = validate_refine_edit(edit("create", "memory", Some("a fact")), 2000);
        match v {
            ValidatedEdit::Rejected(r) => {
                assert!(!r.applied);
                assert!(r.error.unwrap().contains("auto-memory"));
            }
            ValidatedEdit::Accepted(_) => panic!("memory kind must be rejected"),
        }
    }

    #[test]
    fn validate_refine_edit_rejects_unknown_action() {
        let v = validate_refine_edit(edit("archive", "prompt", None), 2000);
        assert!(matches!(v, ValidatedEdit::Rejected(_)));
    }

    #[test]
    fn validate_refine_edit_clamps_content_to_max_chars() {
        let long = "x".repeat(50);
        let v = validate_refine_edit(edit("create", "skill", Some(&long)), 10);
        match v {
            ValidatedEdit::Accepted(e) => {
                let content = e.content.unwrap();
                assert!(content.chars().count() <= 10);
                assert!(content.ends_with('…'));
            }
            ValidatedEdit::Rejected(_) => panic!("valid edit must be accepted"),
        }
    }

    #[test]
    fn validate_refine_edit_leaves_short_content_untouched() {
        let v = validate_refine_edit(edit("update", "prompt", Some("short")), 2000);
        match v {
            ValidatedEdit::Accepted(e) => assert_eq!(e.content.as_deref(), Some("short")),
            ValidatedEdit::Rejected(_) => panic!("valid edit must be accepted"),
        }
    }

    // --- merge_validated_results ------------------------------------------------------------

    #[test]
    fn merge_validated_results_preserves_original_order() {
        let validated = vec![
            ValidatedEdit::Rejected(reject_edit(edit("bogus", "prompt", None), "bad".into())),
            ValidatedEdit::Accepted(edit("create", "prompt", Some("a"))),
            ValidatedEdit::Rejected(reject_edit(edit("create", "memory", None), "bad2".into())),
            ValidatedEdit::Accepted(edit("create", "skill", Some("b"))),
        ];
        let applied_from_store = vec![applied("id-1"), applied("id-2")];
        let merged = merge_validated_results(validated, applied_from_store);

        assert_eq!(merged.len(), 4);
        assert!(!merged[0].applied);
        assert_eq!(merged[0].error.as_deref(), Some("bad"));
        assert!(merged[1].applied);
        assert_eq!(merged[1].id, "id-1");
        assert!(!merged[2].applied);
        assert_eq!(merged[2].error.as_deref(), Some("bad2"));
        assert!(merged[3].applied);
        assert_eq!(merged[3].id, "id-2");
    }

    // --- should_auto_refine_turns -----------------------------------------------------------

    #[test]
    fn should_auto_refine_turns_fires_at_the_interval() {
        assert!(!should_auto_refine_turns(19, 20));
        assert!(should_auto_refine_turns(20, 20));
        assert!(
            should_auto_refine_turns(25, 20),
            "still fires once past the interval"
        );
    }

    #[test]
    fn should_auto_refine_turns_zero_interval_never_fires() {
        assert!(!should_auto_refine_turns(0, 0));
        assert!(!should_auto_refine_turns(1_000, 0));
    }

    // --- scope helpers -----------------------------------------------------------------------

    #[test]
    fn scope_helpers_format_as_the_store_expects() {
        assert_eq!(session_scope("abc"), "session:abc");
        assert_eq!(project_scope("/repo"), "project:/repo");
    }

    // --- payload assembly (order + read-only framing) ----------------------------------------

    #[test]
    fn refine_payload_entries_orders_trajectory_then_overview_then_history() {
        let trajectory = vec![Message::user("hello")];
        let overview = vec![HarnessEntry {
            id: "e1".into(),
            scope: "global".into(),
            kind: "prompt".into(),
            title: "note".into(),
            content: "content".into(),
            source: "refine".into(),
            version: 1,
            created_at: 0,
            updated_at: 0,
        }];
        let history = vec![];
        let entries = refine_payload_entries(&trajectory, &overview, "session:s1", &history);
        let joined = entries.join("\n");
        let traj_idx = joined.find("hello").unwrap();
        let overview_idx = joined.find("id=e1").unwrap();
        let history_idx = joined.find("recent refinement history").unwrap();
        assert!(traj_idx < overview_idx);
        assert!(overview_idx < history_idx);
        assert!(joined.contains("READ-ONLY"));
    }
}
