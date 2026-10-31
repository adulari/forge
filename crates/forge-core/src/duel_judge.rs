//! `/duel --judge`: an optional model pass that grades the finished candidates and crowns one.
//!
//! The judge sees anonymised candidates (letters, never model names) as a diffstat, a test badge
//! and a capped diff, in an order rotated by the task so no model owns the first slot. It returns
//! a ranking with a score and a critique per candidate plus one rationale. A judge that fails or
//! answers nonsense never blocks the duel: the report just carries `judge_error` and the person
//! picks by hand as before.

use std::path::Path;

use forge_provider::StreamEvent;
use forge_types::{Message, TaskTier};

use crate::duel::DuelCandidate;
use crate::subagent::AgentCtx;

/// Largest diff, in bytes, shown to the judge per candidate.
const MAX_DIFF_BYTES: usize = 12_000;

/// Where the judge model comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JudgeChoice {
    /// A cheap-tier model, or the session pin when there is one.
    Auto,
    Model(String),
}

/// A parsed `/duel` command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuelSpec {
    pub task: String,
    /// Candidate count; `None` takes the default.
    pub n: Option<usize>,
    /// Explicit candidate models. The same model may repeat.
    pub models: Vec<String>,
    pub judge: Option<JudgeChoice>,
}

impl DuelSpec {
    pub fn plain(task: &str) -> Self {
        Self {
            task: task.to_string(),
            n: None,
            models: Vec::new(),
            judge: None,
        }
    }

    /// Parse `[--judge[=MODEL]] [--n N] [--models a,b,c] <task>`. Flags come first; the first
    /// word that is not a flag starts the task.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut spec = Self::plain("");
        let mut rest = text.trim();
        loop {
            let (word, after) = match rest.split_once(char::is_whitespace) {
                Some((w, a)) => (w, a.trim_start()),
                None => (rest, ""),
            };
            if word == "--judge" {
                spec.judge = Some(JudgeChoice::Auto);
            } else if let Some(model) = word.strip_prefix("--judge=") {
                if model.is_empty() {
                    return Err("--judge= needs a model id".to_string());
                }
                spec.judge = Some(JudgeChoice::Model(model.to_string()));
            } else if word == "--n" || word.starts_with("--n=") {
                let (value, after) = if let Some(v) = word.strip_prefix("--n=") {
                    (v.to_string(), after)
                } else {
                    let (v, a) = match after.split_once(char::is_whitespace) {
                        Some((v, a)) => (v, a.trim_start()),
                        None => (after, ""),
                    };
                    (v.to_string(), a)
                };
                let n: usize = value
                    .parse()
                    .map_err(|_| format!("--n needs a number, got `{value}`"))?;
                spec.n = Some(n);
                rest = after;
                continue;
            } else if word == "--models" || word.starts_with("--models=") {
                let (value, after) = if let Some(v) = word.strip_prefix("--models=") {
                    (v.to_string(), after)
                } else {
                    let (v, a) = match after.split_once(char::is_whitespace) {
                        Some((v, a)) => (v, a.trim_start()),
                        None => (after, ""),
                    };
                    (v.to_string(), a)
                };
                spec.models = value
                    .split(',')
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .map(str::to_string)
                    .collect();
                if spec.models.is_empty() {
                    return Err("--models needs a comma-separated list of model ids".to_string());
                }
                rest = after;
                continue;
            } else if word.starts_with("--") {
                return Err(format!(
                    "unknown /duel flag `{word}` (flags: --judge[=MODEL], --n N, --models a,b,c)"
                ));
            } else {
                break;
            }
            rest = after;
        }
        spec.task = rest.to_string();
        if spec.task.is_empty() {
            return Err("usage: /duel [--judge[=MODEL]] [--n N] [--models a,b,c] <task>".into());
        }
        if !spec.models.is_empty() {
            match spec.n {
                Some(n) if n != spec.models.len() => {
                    return Err(format!(
                        "--n {n} disagrees with the {} models listed",
                        spec.models.len()
                    ))
                }
                _ => spec.n = Some(spec.models.len()),
            }
        }
        if let Some(n) = spec.n {
            if !(2..=crate::duel::MAX_CANDIDATES).contains(&n) {
                return Err(format!(
                    "a duel needs 2 to {} candidates, not {n}",
                    crate::duel::MAX_CANDIDATES
                ));
            }
        }
        Ok(spec)
    }
}

/// One candidate's place in the judge's ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    /// Index into `DuelReport::candidates`.
    pub index: usize,
    /// 1 is best.
    pub rank: usize,
    /// 0 to 10.
    pub score: f64,
    pub critique: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JudgeVerdict {
    pub judge_model: String,
    /// Best first, one entry per candidate.
    pub ranking: Vec<Judgement>,
    pub rationale: String,
    pub cost_usd: f64,
}

impl JudgeVerdict {
    pub fn winner(&self) -> Option<usize> {
        self.ranking.first().map(|j| j.index)
    }

    pub fn judgement_for(&self, index: usize) -> Option<&Judgement> {
        self.ranking.iter().find(|j| j.index == index)
    }
}

fn label(slot: usize) -> char {
    (b'A' + slot as u8) as char
}

/// The candidate indices in the order the judge sees them: rotated by a hash of the task.
pub(crate) fn presentation_order(task: &str, n: usize) -> Vec<usize> {
    let h = task
        .bytes()
        .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
    let shift = (h % n.max(1) as u64) as usize;
    (0..n).map(|i| (i + shift) % n).collect()
}

fn cap_diff(diff: &str) -> String {
    if diff.len() <= MAX_DIFF_BYTES {
        return diff.to_string();
    }
    let mut end = MAX_DIFF_BYTES;
    while !diff.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n... diff truncated ({} more bytes)",
        &diff[..end],
        diff.len() - end
    )
}

fn branch_diff(repo_root: &Path, branch: &str) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["diff", "HEAD", branch])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

pub(crate) fn build_messages(
    task: &str,
    candidates: &[DuelCandidate],
    diffs: &[String],
    order: &[usize],
) -> Vec<Message> {
    let system = "You are the judge of a coding arena. Several anonymous candidates implemented \
        the same task in separate working copies. Grade them on correctness against the task, \
        test results, scope discipline and code quality. Failing tests weigh heavily. Reply with \
        ONLY a JSON object: {\"ranking\":[{\"label\":\"B\",\"score\":8,\"critique\":\"one or two \
        sentences\"}],\"rationale\":\"why the first one wins, in two sentences\"}. List every \
        candidate exactly once, best first. Scores run 0 to 10.";
    let mut user = format!("Task:\n{task}\n");
    for (slot, &idx) in order.iter().enumerate() {
        let c = &candidates[idx];
        let tests = match c.tests_passed {
            Some(true) => "passed",
            Some(false) => "FAILED",
            None => "not run",
        };
        let diff = diffs.get(idx).map_or("", String::as_str);
        let diff = if diff.trim().is_empty() {
            "(no changes)".to_string()
        } else {
            cap_diff(diff)
        };
        user.push_str(&format!(
            "\n=== Candidate {} ===\n{} files, +{} -{}; tests {tests}\n{diff}\n",
            label(slot),
            c.files_changed,
            c.added,
            c.removed,
        ));
    }
    vec![Message::system(system), Message::user(&user)]
}

fn json_object(raw: &str) -> Option<serde_json::Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    serde_json::from_str(raw.get(start..=end)?).ok()
}

/// The slot a judge's label names: a single letter, optionally written as `Candidate B`. Anything
/// else is rejected rather than read by its first character, which turned `Candidate B` into C.
fn label_slot(label: &str, n: usize) -> Option<usize> {
    let trimmed = label.trim();
    let word = match trimmed.get(..9) {
        Some(head) if head.eq_ignore_ascii_case("candidate") => trimmed[9..].trim(),
        _ => trimmed,
    };
    let mut chars = word.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if chars.next().is_some() || !letter.is_ascii_uppercase() {
        return None;
    }
    let slot = (letter as usize) - ('A' as usize);
    (slot < n).then_some(slot)
}

/// Turn the judge's reply into a full ranking. Labels the judge skipped are appended last with a
/// zero score so every candidate has a row; an unknown label or an empty ranking is an error.
pub(crate) fn parse_verdict(
    raw: &str,
    order: &[usize],
) -> Result<(Vec<Judgement>, String), String> {
    let value = json_object(raw).ok_or("the judge did not answer with JSON")?;
    let entries = value
        .get("ranking")
        .and_then(|r| r.as_array())
        .ok_or("the judge's reply has no ranking")?;
    let mut ranking: Vec<Judgement> = Vec::new();
    for entry in entries {
        let slot = entry
            .get("label")
            .and_then(|l| l.as_str())
            .and_then(|l| label_slot(l, order.len()))
            .ok_or("the judge ranked an unknown candidate")?;
        let index = order[slot];
        if ranking.iter().any(|j| j.index == index) {
            continue;
        }
        ranking.push(Judgement {
            index,
            rank: ranking.len() + 1,
            score: entry
                .get("score")
                .and_then(|s| s.as_f64())
                .unwrap_or(0.0)
                .clamp(0.0, 10.0),
            critique: clean_line(entry.get("critique").and_then(|c| c.as_str()).unwrap_or("")),
        });
    }
    if ranking.is_empty() {
        return Err("the judge ranked nobody".to_string());
    }
    for &index in order {
        if !ranking.iter().any(|j| j.index == index) {
            ranking.push(Judgement {
                index,
                rank: ranking.len() + 1,
                score: 0.0,
                critique: "not ranked by the judge".to_string(),
            });
        }
    }
    let rationale = clean_line(
        value
            .get("rationale")
            .and_then(|r| r.as_str())
            .unwrap_or(""),
    );
    Ok((ranking, rationale))
}

/// One line with no ` · `, so a critique survives the picker row's field separator.
pub fn clean_line(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" · ", " - ")
}

fn auto_model(ctx: &AgentCtx, fallback: &str) -> String {
    if let Some(pin) = ctx.effective_pin.as_ref().and_then(|p| p.first()) {
        return pin.clone();
    }
    ctx.config
        .candidates_for(TaskTier::Trivial)
        .into_iter()
        .filter(|m| !forge_mesh::catalog::is_subscription(m))
        .find(|m| forge_config::has_api_key(forge_config::provider_of(m)))
        .unwrap_or_else(|| fallback.to_string())
}

/// Grade `candidates` with one model call. `fallback_model` is used by [`JudgeChoice::Auto`] when
/// no cheap model has a key.
pub async fn judge(
    ctx: &AgentCtx,
    parent_id: &str,
    task: &str,
    candidates: &[DuelCandidate],
    choice: &JudgeChoice,
    fallback_model: &str,
) -> Result<JudgeVerdict, String> {
    if candidates.len() < 2 {
        return Err("fewer than two finished candidates to compare".to_string());
    }
    let model = match choice {
        JudgeChoice::Model(m) => m.clone(),
        JudgeChoice::Auto => auto_model(ctx, fallback_model),
    };
    let diffs: Vec<String> = {
        let repo_root = ctx.repo_root.clone();
        let branches: Vec<String> = candidates.iter().map(|c| c.branch.clone()).collect();
        tokio::task::spawn_blocking(move || {
            branches
                .iter()
                .map(|b| branch_diff(&repo_root, b))
                .collect()
        })
        .await
        .map_err(|e| format!("diff task failed: {e}"))?
    };
    let order = presentation_order(task, candidates.len());
    let messages = build_messages(task, candidates, &diffs, &order);
    let mut sink = |_ev: StreamEvent| {};
    let reply = ctx
        .provider
        .complete(&model, &messages, &[], &mut sink)
        .await
        .map_err(|e| format!("judge {model} failed: {}", e.reason()))?;
    // Billed whether or not the reply parses, so it counts toward the session and budget caps.
    let _ =
        ctx.store
            .record_side_call_usage_for(parent_id, "duel-judge", Some(&model), &reply.usage);
    let (ranking, rationale) = parse_verdict(&reply.content, &order)?;
    Ok(JudgeVerdict {
        judge_model: model.clone(),
        ranking,
        rationale,
        cost_usd: ctx.pricing.cost_for_usage(&model, &reply.usage),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(model: &str, tests: Option<bool>) -> DuelCandidate {
        DuelCandidate {
            model: model.to_string(),
            child_id: format!("c-{model}"),
            branch: format!("forge/{model}"),
            files_changed: 1,
            added: 3,
            removed: 1,
            tests_passed: tests,
            duration_ms: 1000,
            cost_usd: 0.01,
            summary: format!("{model} says hello"),
            ok: true,
        }
    }

    #[test]
    fn parse_reads_flags_then_the_task() {
        let s = DuelSpec::parse("--judge --n 3 fix the bug").unwrap();
        assert_eq!(s.task, "fix the bug");
        assert_eq!(s.judge, Some(JudgeChoice::Auto));
        assert_eq!(s.n, Some(3));
        let s = DuelSpec::parse("--judge=a::b --models x::1,x::2,y::3 do it --not-a-flag").unwrap();
        assert_eq!(s.judge, Some(JudgeChoice::Model("a::b".into())));
        assert_eq!(s.models, vec!["x::1", "x::2", "y::3"]);
        assert_eq!(s.n, Some(3));
        assert_eq!(s.task, "do it --not-a-flag");
    }

    #[test]
    fn parse_rejects_bad_input() {
        assert!(DuelSpec::parse("").is_err());
        assert!(DuelSpec::parse("--judge").is_err());
        assert!(DuelSpec::parse("--n 9 task").is_err());
        assert!(DuelSpec::parse("--n 1 task").is_err());
        assert!(DuelSpec::parse("--n x task").is_err());
        assert!(DuelSpec::parse("--wat task").is_err());
        assert!(DuelSpec::parse("--n 3 --models a,b task").is_err());
        assert!(DuelSpec::parse("--judge= task").is_err());
    }

    #[test]
    fn plain_text_is_the_task() {
        let s = DuelSpec::parse("  add pagination  ").unwrap();
        assert_eq!(s, DuelSpec::plain("add pagination"));
    }

    #[test]
    fn order_is_a_rotation_of_every_index() {
        for task in ["a", "fix it", "something longer"] {
            let mut o = presentation_order(task, 3);
            o.sort_unstable();
            assert_eq!(o, vec![0, 1, 2]);
        }
    }

    #[test]
    fn prompt_hides_model_names_and_caps_diffs() {
        let cands = vec![
            cand("secret-model-one", Some(true)),
            cand("other", Some(false)),
        ];
        let big = "x".repeat(MAX_DIFF_BYTES + 500);
        let diffs = vec![big, String::new()];
        let msgs = build_messages("do it", &cands, &diffs, &[1, 0]);
        let all = format!("{msgs:?}");
        assert!(!all.contains("secret-model-one"));
        assert!(all.contains("Candidate A"));
        assert!(all.contains("diff truncated"));
        assert!(all.contains("(no changes)"));
        assert!(all.contains("FAILED"));
    }

    #[test]
    fn verdict_maps_labels_back_through_the_order() {
        let raw = "```json\n{\"ranking\":[{\"label\":\"B\",\"score\":9,\"critique\":\"clean · small\"},\
            {\"label\":\"a\",\"score\":4,\"critique\":\"misses edge\"}],\"rationale\":\"B is tidy\"}\n```";
        let (ranking, rationale) = parse_verdict(raw, &[2, 0, 1]).unwrap();
        assert_eq!(ranking[0].index, 0);
        assert_eq!(ranking[0].rank, 1);
        assert_eq!(ranking[0].critique, "clean - small");
        assert_eq!(ranking[1].index, 2);
        assert_eq!(ranking[2].index, 1);
        assert_eq!(ranking[2].score, 0.0);
        assert_eq!(rationale, "B is tidy");
    }

    #[test]
    fn verdict_rejects_garbage() {
        assert!(parse_verdict("no json here", &[0, 1]).is_err());
        assert!(parse_verdict("{\"ranking\":[]}", &[0, 1]).is_err());
        assert!(parse_verdict("{\"ranking\":[{\"label\":\"Z\"}]}", &[0, 1]).is_err());
    }

    #[test]
    fn candidate_prefixed_labels_map_to_their_letter() {
        let raw = "{\"ranking\":[{\"label\":\"Candidate B\",\"score\":9},{\"label\":\"candidate c\",\"score\":5},{\"label\":\"A\",\"score\":1}]}";
        let (ranking, _) = parse_verdict(raw, &[0, 1, 2]).unwrap();
        assert_eq!(
            ranking.iter().map(|j| j.index).collect::<Vec<_>>(),
            vec![1, 2, 0]
        );
        assert!(parse_verdict("{\"ranking\":[{\"label\":\"Candidates\"}]}", &[0, 1]).is_err());
        assert!(parse_verdict("{\"ranking\":[{\"label\":\"BC\"}]}", &[0, 1, 2]).is_err());
        assert!(parse_verdict("{\"ranking\":[{\"label\":\"C\"}]}", &[0, 1]).is_err());
    }

    #[test]
    fn duplicate_labels_count_once() {
        let raw = "{\"ranking\":[{\"label\":\"A\",\"score\":7},{\"label\":\"A\",\"score\":1}]}";
        let (ranking, _) = parse_verdict(raw, &[0, 1]).unwrap();
        assert_eq!(ranking.len(), 2);
        assert_eq!(ranking[0].score, 7.0);
    }

    #[test]
    fn winner_is_the_first_ranked() {
        let (ranking, rationale) = parse_verdict(
            "{\"ranking\":[{\"label\":\"B\",\"score\":8},{\"label\":\"A\",\"score\":2}],\"rationale\":\"r\"}",
            &[0, 1],
        )
        .unwrap();
        let v = JudgeVerdict {
            judge_model: "m".into(),
            ranking,
            rationale,
            cost_usd: 0.0,
        };
        assert_eq!(v.winner(), Some(1));
        assert_eq!(v.judgement_for(0).map(|j| j.rank), Some(2));
    }

    struct Scripted(&'static str);
    #[async_trait::async_trait]
    impl forge_provider::Provider for Scripted {
        async fn complete(
            &self,
            _model: &str,
            _messages: &[forge_types::Message],
            _tools: &[forge_provider::ToolSpec],
            _on_event: &mut forge_provider::EventSink<'_>,
        ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
            Ok(forge_provider::ModelResponse {
                reasoning: String::new(),
                reasoning_items: Vec::new(),
                content: self.0.to_string(),
                tool_calls: vec![],
                usage: forge_types::Usage {
                    input_tokens: 1234,
                    output_tokens: 56,
                    ..forge_types::Usage::default()
                },
                quotas: Vec::new(),
            })
        }
    }

    struct OneModel;
    #[async_trait::async_trait]
    impl forge_mesh::Router for OneModel {
        async fn route(
            &self,
            _p: &str,
            _i: bool,
            _b: forge_mesh::BudgetState,
            _h: &forge_types::ModelHealth,
            _q: &forge_types::SubscriptionQuota,
            _e: Option<forge_types::EffortLevel>,
            _pc: &forge_types::ProjectContext,
        ) -> forge_mesh::RoutingDecision {
            unreachable!("the judge never routes")
        }
    }

    #[tokio::test]
    async fn judge_spend_lands_on_the_parent_session() {
        let config = forge_config::Config::default();
        let store = std::sync::Arc::new(forge_store::Store::open_in_memory().unwrap());
        let parent = store.create_session("/tmp", "Default").unwrap();
        let ctx = AgentCtx {
            provider: std::sync::Arc::new(Scripted(
                "{\"ranking\":[{\"label\":\"A\",\"score\":5},{\"label\":\"B\",\"score\":4}]}",
            )),
            router: std::sync::Arc::new(OneModel),
            store: store.clone(),
            pricing: forge_mesh::pricing::Pricing::from_config(&config),
            config,
            mode: forge_types::PermissionMode::default(),
            rules: Vec::new(),
            depth: 0,
            max_depth: 1,
            agents: std::sync::Arc::new(std::collections::HashMap::new()),
            worktree_root: None,
            repo_root: std::path::PathBuf::from("/nonexistent"),
            effective_pin: None,
            inherit_pin: true,
            asker: None,
        };
        let cands = vec![cand("a", Some(true)), cand("b", Some(true))];
        let v = judge(
            &ctx,
            &parent,
            "t",
            &cands,
            &JudgeChoice::Model("m::j".into()),
            "m::x",
        )
        .await
        .unwrap();
        assert_eq!(v.ranking.len(), 2);
        let rows = store.session_usage_by_model(&parent).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].model.as_deref(), Some("m::j"));
        assert_eq!(
            (rows[0].usage.input_tokens, rows[0].usage.output_tokens),
            (1234, 56)
        );
    }
}
