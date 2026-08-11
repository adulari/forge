//! `forge replay` — auditable, reproducible reconstruction of a past session from the
//! persisted record (docs/features/session-replay.md). One id prints the turn-by-turn
//! transcript with the model, tokens, cost, and wall-clock time of each turn; two ids diff
//! their summaries so you can see what changed between two runs of a similar task.
//!
//! This module is pure over [`ReplayEntry`] slices so the summary/diff logic is unit-tested
//! without a database; the CLI command resolves ids and prints what these functions return.

use chrono::{Local, TimeZone};
use forge_store::ReplayEntry;
use forge_types::Role;

/// Aggregate stats over one session's replay entries.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplaySummary {
    /// Number of user prompts (the unit a human thinks of as a "turn").
    pub prompts: usize,
    pub messages: usize,
    pub total_cost: f64,
    pub total_in: i64,
    pub total_out: i64,
    /// Distinct models, in first-seen order.
    pub models: Vec<String>,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl ReplaySummary {
    pub fn duration_secs(&self) -> Option<i64> {
        match (self.started_at, self.ended_at) {
            (Some(s), Some(e)) => Some(e - s),
            _ => None,
        }
    }
}

pub fn summarize(entries: &[ReplayEntry]) -> ReplaySummary {
    let mut models: Vec<String> = Vec::new();
    let mut total_cost = 0.0;
    let (mut total_in, mut total_out) = (0i64, 0i64);
    let mut prompts = 0;
    for e in entries {
        if e.role == Role::User {
            prompts += 1;
        }
        if let Some(m) = &e.model {
            if !models.iter().any(|x| x == m) {
                models.push(m.clone());
            }
        }
        total_cost += e.cost_usd.unwrap_or(0.0);
        total_in += e.input_tokens.unwrap_or(0);
        total_out += e.output_tokens.unwrap_or(0);
    }
    ReplaySummary {
        prompts,
        messages: entries.len(),
        total_cost,
        total_in,
        total_out,
        models,
        started_at: entries.first().map(|e| e.created_at),
        ended_at: entries.last().map(|e| e.created_at),
    }
}

/// A summary-level comparison of two sessions: the "did this run cost more / use a different
/// model / take more turns than that run" audit question.
#[derive(Debug, Clone)]
pub struct SessionDiff {
    pub a: ReplaySummary,
    pub b: ReplaySummary,
    /// `b - a` (positive = b spent more).
    pub cost_delta: f64,
    /// `b - a` prompt count.
    pub prompt_delta: i64,
    pub models_only_a: Vec<String>,
    pub models_only_b: Vec<String>,
}

pub fn diff(a: &[ReplayEntry], b: &[ReplayEntry]) -> SessionDiff {
    let sa = summarize(a);
    let sb = summarize(b);
    let only_a = sa
        .models
        .iter()
        .filter(|m| !sb.models.contains(m))
        .cloned()
        .collect();
    let only_b = sb
        .models
        .iter()
        .filter(|m| !sa.models.contains(m))
        .cloned()
        .collect();
    SessionDiff {
        cost_delta: sb.total_cost - sa.total_cost,
        prompt_delta: sb.prompts as i64 - sa.prompts as i64,
        models_only_a: only_a,
        models_only_b: only_b,
        a: sa,
        b: sb,
    }
}

fn fmt_time(epoch: i64) -> String {
    Local
        .timestamp_opt(epoch, 0)
        .single()
        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| epoch.to_string())
}

fn role_label(role: &Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
    }
}

/// Truncate to `max` chars on a char boundary, appending an ellipsis when cut.
fn clip(s: &str, max: usize) -> String {
    let one_line = s.replace('\n', " ");
    if one_line.chars().count() <= max {
        one_line
    } else {
        let head: String = one_line.chars().take(max).collect();
        format!("{head}…")
    }
}

/// Turn-by-turn transcript for a single session.
pub fn render_transcript(id: &str, entries: &[ReplayEntry]) -> String {
    let s = summarize(entries);
    let mut out = String::new();
    out.push_str(&format!("session {id}\n"));
    if let Some(start) = s.started_at {
        match s.duration_secs() {
            Some(d) if d > 0 => {
                out.push_str(&format!("  started {} · {d}s elapsed\n", fmt_time(start)))
            }
            _ => out.push_str(&format!("  started {}\n", fmt_time(start))),
        }
    }
    out.push_str(&format!(
        "  {} prompts · {} messages · {} models · ${:.4} · {}↑/{}↓ tok\n\n",
        s.prompts,
        s.messages,
        s.models.len(),
        s.total_cost,
        s.total_in,
        s.total_out,
    ));
    for e in entries {
        let meta = match (&e.model, e.cost_usd) {
            (Some(m), Some(c)) => format!("  [{m} · ${c:.4}]"),
            (Some(m), None) => format!("  [{m}]"),
            _ => String::new(),
        };
        out.push_str(&format!(
            "{:>10}  {}{meta}\n",
            role_label(&e.role),
            clip(&e.content, 100)
        ));
        for tc in &e.tool_calls {
            out.push_str(&format!(
                "            ↳ {}({})\n",
                tc.name,
                clip(&tc.args.to_string(), 60)
            ));
        }
    }
    out
}

/// Per-turn content diff: align corresponding assistant turns and show where the text diverges.
/// Useful when two sessions worked on the same task but took different paths.
pub fn render_turn_diff(id_a: &str, id_b: &str, a: &[ReplayEntry], b: &[ReplayEntry]) -> String {
    let mut out = String::new();
    let asst_a: Vec<&ReplayEntry> = a.iter().filter(|e| e.role == Role::Assistant).collect();
    let asst_b: Vec<&ReplayEntry> = b.iter().filter(|e| e.role == Role::Assistant).collect();
    out.push_str(&format!(
        "turn diff  {id_a}  →  {id_b}  ({} vs {} assistant turns)\n\n",
        asst_a.len(),
        asst_b.len()
    ));
    let n = asst_a.len().max(asst_b.len());
    for i in 0..n {
        let ca = asst_a.get(i).map(|e| clip(&e.content, 120));
        let cb = asst_b.get(i).map(|e| clip(&e.content, 120));
        match (ca, cb) {
            (Some(a), Some(b)) if a == b => {
                out.push_str(&format!("  turn {:<3}  [identical]\n", i + 1));
            }
            (Some(a), Some(b)) => {
                out.push_str(&format!("  turn {:<3}  A: {a}\n", i + 1));
                out.push_str(&format!("         B: {b}\n"));
            }
            (Some(a), None) => {
                out.push_str(&format!("  turn {:<3}  A: {a}\n", i + 1));
                out.push_str("         B: <none>\n");
            }
            (None, Some(b)) => {
                out.push_str(&format!("  turn {:<3}  A: <none>\n", i + 1));
                out.push_str(&format!("         B: {b}\n"));
            }
            (None, None) => {}
        }
    }
    out
}

/// JSON export of a single session's replay (transcript + summary), for external auditing.
///
/// Emits a JSON object: `{ session_id, summary, turns }`.
/// Each turn carries `seq`, `role`, `created_at`, `content`, optional `model`, token counts,
/// `cost_usd`, and `tool_calls`.
pub fn render_json(id: &str, entries: &[ReplayEntry]) -> String {
    let s = summarize(entries);
    let turns: Vec<serde_json::Value> = entries
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "role": role_label(&e.role),
                "created_at": e.created_at,
                "content": e.content,
                "model": e.model,
                "input_tokens": e.input_tokens,
                "output_tokens": e.output_tokens,
                "cost_usd": e.cost_usd,
                "tool_calls": e.tool_calls.iter().map(|tc| serde_json::json!({
                    "id": tc.id,
                    "name": tc.name,
                    "args": tc.args,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let obj = serde_json::json!({
        "session_id": id,
        "summary": {
            "prompts": s.prompts,
            "messages": s.messages,
            "total_cost_usd": s.total_cost,
            "total_input_tokens": s.total_in,
            "total_output_tokens": s.total_out,
            "models": s.models,
            "started_at": s.started_at,
            "ended_at": s.ended_at,
            "duration_secs": s.duration_secs(),
        },
        "turns": turns,
    });
    serde_json::to_string_pretty(&obj).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}

/// Escape the five HTML-significant characters. Message/tool content is untrusted (it's whatever
/// the user typed or the model/tools produced), so every dynamic value `render_html` interpolates
/// goes through this — including the session id, which is normally a generated UUID but costs
/// nothing to escape defensively too.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

const HTML_CSS: &str = r#"
:root { color-scheme: light dark; }
* { box-sizing: border-box; }
body {
    font-family: ui-monospace, "SF Mono", "Cascadia Code", Menlo, Consolas, monospace;
    font-size: 14px;
    margin: 0;
    padding: 2rem;
    max-width: 960px;
    margin-inline: auto;
    background: #f7f7f8;
    color: #1a1a1a;
    line-height: 1.55;
}
@media (prefers-color-scheme: dark) {
    body { background: #16171b; color: #e4e4e7; }
    .turn { background: #1f2024; border-color: #2c2d33; }
    dd { color: #f0f0f2; }
    .tool-call { background: #17181c; color: #a9b1c2; }
}
header { margin-bottom: 2rem; border-bottom: 1px solid rgba(127,127,127,0.35); padding-bottom: 1rem; }
h1 { font-size: 1.05rem; margin: 0 0 0.9rem; font-weight: 600; }
dl.summary { display: flex; flex-wrap: wrap; gap: 0.5rem 1.75rem; margin: 0; padding: 0; }
dl.summary > div { display: flex; gap: 0.4rem; align-items: baseline; }
dt { opacity: 0.6; }
dd { margin: 0; font-weight: 600; }
main { display: flex; flex-direction: column; gap: 0.75rem; }
.turn {
    border: 1px solid rgba(127,127,127,0.25);
    border-radius: 8px;
    padding: 0.65rem 0.9rem;
    background: #ffffff;
}
.turn-header { display: flex; gap: 0.75rem; align-items: baseline; margin-bottom: 0.35rem; }
.role { font-weight: 700; text-transform: uppercase; font-size: 0.72rem; letter-spacing: 0.04em; padding: 0.1rem 0.4rem; border-radius: 4px; }
.role-user { background: rgba(90,140,255,0.18); color: #3a67d4; }
.role-assistant { background: rgba(90,200,140,0.18); color: #1f8a56; }
.role-tool { background: rgba(200,150,60,0.18); color: #9a6a10; }
.role-system { background: rgba(150,150,150,0.2); color: #6b6b6b; }
@media (prefers-color-scheme: dark) {
    .role-user { color: #8fb2ff; }
    .role-assistant { color: #6fdba3; }
    .role-tool { color: #e0b45c; }
    .role-system { color: #a8a8a8; }
}
.meta { opacity: 0.55; font-size: 0.78rem; }
.content { margin: 0; white-space: pre-wrap; word-break: break-word; font: inherit; }
.tool-call { margin-top: 0.4rem; padding: 0.3rem 0.5rem; border-radius: 5px; background: #f0f0f2; font-size: 0.82rem; white-space: pre-wrap; word-break: break-word; color: #55596b; }
footer { margin-top: 2rem; opacity: 0.5; font-size: 0.78rem; }
"#;

/// Self-contained HTML render of a session transcript — `forge replay <id> --html <path>` and the
/// TUI's `/export`. One file: session metadata header (id, started/ended, models, cost, tokens)
/// then the turn sequence as user/assistant/tool cards, inline CSS only, no external assets,
/// dark-friendly via `prefers-color-scheme`. All dynamic content is HTML-escaped — message and
/// tool-call content is untrusted text.
pub fn render_html(id: &str, entries: &[ReplayEntry]) -> String {
    let s = summarize(entries);
    let started = s
        .started_at
        .map(fmt_time)
        .unwrap_or_else(|| "—".to_string());
    let ended = s.ended_at.map(fmt_time).unwrap_or_else(|| "—".to_string());
    let models = if s.models.is_empty() {
        "—".to_string()
    } else {
        s.models
            .iter()
            .map(|m| escape_html(m))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let mut turns = String::new();
    for e in entries {
        let role = role_label(&e.role);
        let meta = match (&e.model, e.cost_usd) {
            (Some(m), Some(c)) => format!(" · {} · ${c:.4}", escape_html(m)),
            (Some(m), None) => format!(" · {}", escape_html(m)),
            _ => String::new(),
        };
        let mut tool_html = String::new();
        for tc in &e.tool_calls {
            tool_html.push_str(&format!(
                "<pre class=\"tool-call\">↳ {}({})</pre>\n",
                escape_html(&tc.name),
                escape_html(&tc.args.to_string()),
            ));
        }
        turns.push_str(&format!(
            "<article class=\"turn\" data-role=\"{role}\">\n\
             <div class=\"turn-header\"><span class=\"role role-{role}\">{role}</span><span class=\"meta\">{}{meta}</span></div>\n\
             <pre class=\"content\">{}</pre>\n\
             {tool_html}\
             </article>\n",
            escape_html(&fmt_time(e.created_at)),
            escape_html(&e.content),
        ));
    }

    let esc_id = escape_html(id);
    format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>forge session {esc_id}</title>\n\
<style>{HTML_CSS}</style>\n\
</head>\n\
<body>\n\
<header>\n\
<h1>session {esc_id}</h1>\n\
<dl class=\"summary\">\n\
<div><dt>started</dt><dd>{started}</dd></div>\n\
<div><dt>ended</dt><dd>{ended}</dd></div>\n\
<div><dt>prompts</dt><dd>{}</dd></div>\n\
<div><dt>messages</dt><dd>{}</dd></div>\n\
<div><dt>models</dt><dd>{models}</dd></div>\n\
<div><dt>cost</dt><dd>${:.4}</dd></div>\n\
<div><dt>tokens</dt><dd>{}↑ / {}↓</dd></div>\n\
</dl>\n\
</header>\n\
<main>\n\
{turns}</main>\n\
<footer>generated by <code>forge replay --html</code> — a self-contained, offline snapshot</footer>\n\
</body>\n\
</html>\n",
        s.prompts, s.messages, s.total_cost, s.total_in, s.total_out,
    )
}

/// The original user prompts of a session, in turn order — the input to re-execution
/// (`forge replay <id> --rerun`). Only `User` turns are real prompts; assistant/tool entries
/// are model output. Blank entries are dropped so an empty turn can't feed the agent an empty
/// prompt.
pub fn user_prompts(entries: &[ReplayEntry]) -> Vec<String> {
    entries
        .iter()
        .filter(|e| e.role == Role::User && !e.content.trim().is_empty())
        .map(|e| e.content.clone())
        .collect()
}

/// Summary-level diff between two sessions.
pub fn render_diff(id_a: &str, id_b: &str, d: &SessionDiff) -> String {
    let mut out = String::new();
    out.push_str(&format!("replay diff  {id_a}  →  {id_b}\n\n"));
    out.push_str(&format!(
        "  prompts   {:>8}  →  {:<8}  ({:+})\n",
        d.a.prompts, d.b.prompts, d.prompt_delta
    ));
    out.push_str(&format!(
        "  cost      ${:>7.4}  →  ${:<7.4}  ({:+.4})\n",
        d.a.total_cost, d.b.total_cost, d.cost_delta
    ));
    out.push_str(&format!(
        "  tokens    {:>4}↑/{:<4}↓  →  {:>4}↑/{:<4}↓\n",
        d.a.total_in, d.a.total_out, d.b.total_in, d.b.total_out
    ));
    if !d.models_only_a.is_empty() {
        out.push_str(&format!(
            "  only in {id_a}: {}\n",
            d.models_only_a.join(", ")
        ));
    }
    if !d.models_only_b.is_empty() {
        out.push_str(&format!(
            "  only in {id_b}: {}\n",
            d.models_only_b.join(", ")
        ));
    }
    if d.models_only_a.is_empty() && d.models_only_b.is_empty() {
        out.push_str("  models    identical\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_types::ToolCall;

    fn entry(
        seq: i64,
        role: Role,
        content: &str,
        model: Option<&str>,
        cost: Option<f64>,
    ) -> ReplayEntry {
        ReplayEntry {
            seq,
            role,
            content: content.to_string(),
            model: model.map(String::from),
            created_at: 1_000 + seq,
            tool_calls: Vec::new(),
            input_tokens: cost.map(|_| 10),
            output_tokens: cost.map(|_| 5),
            cost_usd: cost,
        }
    }

    #[test]
    fn summarize_counts_prompts_cost_and_distinct_models() {
        let entries = vec![
            entry(0, Role::User, "ask one", None, None),
            entry(
                1,
                Role::Assistant,
                "answer",
                Some("openai::gpt-4o"),
                Some(0.02),
            ),
            entry(2, Role::User, "ask two", None, None),
            entry(
                3,
                Role::Assistant,
                "answer",
                Some("openai::gpt-4o"),
                Some(0.03),
            ),
        ];
        let s = summarize(&entries);
        assert_eq!(s.prompts, 2);
        assert_eq!(s.messages, 4);
        assert_eq!(s.models, vec!["openai::gpt-4o"]);
        assert!((s.total_cost - 0.05).abs() < 1e-9);
        assert_eq!(s.duration_secs(), Some(3));
    }

    #[test]
    fn user_prompts_keeps_only_nonblank_user_turns_in_order() {
        let entries = vec![
            entry(0, Role::User, "first task", None, None),
            entry(1, Role::Assistant, "working on it", Some("m"), Some(0.01)),
            entry(2, Role::User, "   ", None, None), // blank turn — dropped
            entry(3, Role::User, "second task", None, None),
        ];
        assert_eq!(user_prompts(&entries), vec!["first task", "second task"]);
    }

    #[test]
    fn diff_reports_cost_and_model_divergence() {
        let a = vec![
            entry(0, Role::User, "task", None, None),
            entry(1, Role::Assistant, "a", Some("openai::gpt-4o"), Some(0.10)),
        ];
        let b = vec![
            entry(0, Role::User, "task", None, None),
            entry(
                1,
                Role::Assistant,
                "b",
                Some("anthropic::claude"),
                Some(0.04),
            ),
        ];
        let d = diff(&a, &b);
        assert!((d.cost_delta + 0.06).abs() < 1e-9, "b is 0.06 cheaper");
        assert_eq!(d.prompt_delta, 0);
        assert_eq!(d.models_only_a, vec!["openai::gpt-4o"]);
        assert_eq!(d.models_only_b, vec!["anthropic::claude"]);
    }

    #[test]
    fn transcript_shows_model_cost_and_tool_calls() {
        let mut e = entry(
            1,
            Role::Assistant,
            "calling a tool",
            Some("openai::gpt-4o"),
            Some(0.02),
        );
        e.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "read_file".into(),
            args: serde_json::json!({ "path": "x" }),
        });
        let out = render_transcript("abc123", &[e]);
        assert!(out.contains("session abc123"));
        assert!(out.contains("openai::gpt-4o · $0.02"));
        assert!(out.contains("↳ read_file"));
    }

    #[test]
    fn render_json_is_valid_json_with_expected_fields() {
        let entries = vec![
            entry(0, Role::User, "hello", None, None),
            entry(
                1,
                Role::Assistant,
                "world",
                Some("openai::gpt-4o"),
                Some(0.01),
            ),
        ];
        let out = render_json("full-id-here", &entries);
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(v["session_id"], "full-id-here");
        assert_eq!(v["summary"]["prompts"], 1);
        assert_eq!(v["summary"]["messages"], 2);
        assert!((v["summary"]["total_cost_usd"].as_f64().unwrap() - 0.01).abs() < 1e-9);
        assert_eq!(v["turns"].as_array().unwrap().len(), 2);
        assert_eq!(v["turns"][0]["role"], "user");
        assert_eq!(v["turns"][1]["model"], "openai::gpt-4o");
    }

    #[test]
    fn render_html_escapes_a_script_tag_inert() {
        let entries = vec![entry(
            0,
            Role::User,
            "<script>alert(1)</script>",
            None,
            None,
        )];
        let out = render_html("abc123", &entries);
        assert!(
            !out.contains("<script>alert(1)</script>"),
            "the raw tag must never appear unescaped in the output"
        );
        assert!(out.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn render_html_escapes_tool_call_name_and_args() {
        let mut e = entry(1, Role::Assistant, "running a tool", Some("m"), None);
        e.tool_calls.push(ToolCall {
            id: "c1".into(),
            name: "<b>evil</b>".into(),
            args: serde_json::json!({ "cmd": "<img src=x onerror=alert(1)>" }),
        });
        let out = render_html("abc123", &[e]);
        assert!(!out.contains("<b>evil</b>"));
        assert!(!out.contains("<img src=x onerror=alert(1)>"));
        assert!(out.contains("&lt;b&gt;evil&lt;/b&gt;"));
    }

    #[test]
    fn render_html_has_one_turn_marker_per_entry() {
        let entries = vec![
            entry(0, Role::User, "first", None, None),
            entry(1, Role::Assistant, "second", Some("m"), Some(0.01)),
            entry(2, Role::User, "third", None, None),
            entry(3, Role::Assistant, "fourth", Some("m"), Some(0.02)),
        ];
        let out = render_html("full-id-here", &entries);
        assert_eq!(
            out.matches("class=\"turn\"").count(),
            entries.len(),
            "one turn card per replay entry"
        );
    }

    #[test]
    fn render_html_is_one_self_contained_well_formed_document() {
        let entries = vec![
            entry(0, Role::User, "hello", None, None),
            entry(
                1,
                Role::Assistant,
                "world",
                Some("openai::gpt-4o"),
                Some(0.01),
            ),
        ];
        let out = render_html("full-id-here", &entries);
        assert!(out.trim_start().starts_with("<!doctype html>"));
        assert!(out.contains("<html"));
        assert!(out.contains("</html>"));
        assert!(out.contains("<style>"), "CSS is inlined, not linked");
        assert!(
            !out.contains("http://") && !out.contains("https://") && !out.contains("<link "),
            "no external assets"
        );
        assert!(out.contains("session full-id-h"));
        assert!(out.contains("openai::gpt-4o"));
        // Balanced enough to be well-formed for this generator's own fixed structure.
        assert_eq!(
            out.matches("<article").count(),
            out.matches("</article>").count()
        );
    }

    #[test]
    fn render_html_on_empty_transcript_still_produces_a_valid_shell() {
        let out = render_html("empty-id", &[]);
        assert!(out.contains("<!doctype html>"));
        assert_eq!(out.matches("class=\"turn\"").count(), 0);
    }
}
