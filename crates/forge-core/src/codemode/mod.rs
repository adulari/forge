//! Codemode: the `run_code` tool (docs/features/codemode.md). The model writes a short JS program
//! that calls Forge's other tools as `await tools.<name>(args)` and returns only what it needs, so
//! one model step can do what otherwise takes several round trips.
//!
//! The program runs in the forge-workflow QuickJS sandbox, which has no filesystem, network or
//! process access of its own. Its only capability is `__tool`, a host function that sends each
//! request back to the session task, where it runs through `Session::invoke_tool` exactly like a
//! normal call (task scope, permissions and prompts, hooks, /undo snapshots, audit) and then
//! through the shared `observe_tool_outcome` bookkeeping. This module holds the pure parts: the
//! spec, argument parsing, the JS wrapper and result rendering. The pump lives in `runner.rs`.

mod catalog;
mod prompt_clock;
mod runner;

use forge_provider::ToolSpec;

/// The virtual tool name the model calls.
pub const RUN_CODE_TOOL: &str = "run_code";

/// Name the prelude's `tools.$search` sends through `__tool`.
pub(crate) const SEARCH_CALL: &str = "$search";

/// Log lines a script may keep; later lines are counted, not stored.
const MAX_LOG_LINES: usize = 200;

/// The spec advertised to direct-path models. Static text, so the sorted catalog stays
/// byte-stable for prompt caching: no per-session tool list goes in here.
pub fn run_code_spec() -> ToolSpec {
    ToolSpec {
        name: RUN_CODE_TOOL.to_string(),
        description: "Run a short JavaScript program that calls your other tools and returns only \
            what you need, in ONE step. Use it to chain several tool calls (search, then read the \
            matches, then edit, then test) or to filter large output down to the lines that \
            matter. Every tool you have is callable as `await tools.<name>({...same JSON \
            arguments...})`; it resolves to the tool's output string and throws an Error when the \
            call is denied or fails to run (a shell command that exits non-zero still resolves; \
            check its `shell: exit N` first line). Tools not listed here are callable by name \
            too; `await tools.$search(\"keywords\")` returns matching tools' names, descriptions \
            and schemas. Calls run one at a time in the order they are made; permission prompts \
            and /undo snapshots work as for normal calls. `console.log(...)` lines are kept. \
            The code runs inside an async function: end it with `return <result>`. There is no \
            fs, fetch, require or timer API; use the tools. Not callable from a script: \
            run_code, run_workflow, spawn_agents, ask_user, present_plan, update_tasks and the \
            other session-control tools."
            .to_string(),
        schema: serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "JS statements run inside an async function; end with `return`. A ```js fenced block is accepted."
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Script budget in seconds (default 300); time waiting on a permission prompt does not count."
                }
            },
            "required": ["code"]
        }),
    }
}

/// The script body and its budget from a `run_code` call. Accepts the code in `code` (or a
/// mistaken `script`), with or without a Markdown fence: models that mangle escaped JSON often
/// do better writing a fenced block.
pub(crate) fn script_from_args(
    args: &serde_json::Value,
    config: &forge_config::codemode::CodemodeConfig,
) -> Result<(String, u64), String> {
    let raw = match args {
        serde_json::Value::String(s) => Some(s.as_str()),
        other => other
            .get("code")
            .or_else(|| other.get("script"))
            .and_then(serde_json::Value::as_str),
    };
    let code = raw.map(strip_fence).unwrap_or_default();
    if code.trim().is_empty() {
        return Err("run_code requires a non-empty `code` string".to_string());
    }
    let timeout = args
        .get("timeout_secs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(config.timeout_secs)
        .clamp(1, config.max_timeout_secs.max(1));
    Ok((code, timeout))
}

/// Strip one surrounding Markdown code fence (```` ```js ```` … ```` ``` ````), if present.
fn strip_fence(raw: &str) -> String {
    let trimmed = raw.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return raw.to_string();
    };
    let Some(body) = rest.strip_suffix("```") else {
        return raw.to_string();
    };
    // The info string (`js`, `javascript`, ...) runs to the first newline.
    match body.split_once('\n') {
        Some((info, code)) if !info.contains(' ') || info.trim().is_empty() => code.to_string(),
        _ => body.to_string(),
    }
}

/// The program wrapped for the engine: an async IIFE (the engine evaluates the script to a
/// function and calls it) whose first line is the whole prelude, so the model's code starts on
/// line 2. The prelude defines `tools`, `console` and `log`, and the epilogue reports the value,
/// the error and the captured logs as one object, so logs survive a thrown error.
pub(crate) fn wrap(code: &str) -> String {
    format!("{}\n{code}\n{}", prelude(), EPILOGUE)
}

fn prelude() -> String {
    [
        "(async () => {",
        "const __logs = []; let __dropped = 0;",
        "const __fmt = (v) => { if (typeof v === 'string') return v; try { const s = JSON.stringify(v); return s === undefined ? String(v) : s; } catch (_) { try { return String(v); } catch (_) { return '[unprintable]'; } } };",
        &format!("const console = {{ log: (...a) => {{ if (__logs.length < {MAX_LOG_LINES}) __logs.push(a.map(__fmt).join(' ')); else __dropped++; }} }};"),
        "console.info = console.log; console.warn = console.log; console.error = console.log; console.debug = console.log;",
        "const log = console.log;",
        // `then` must be undefined or `await tools` would treat the proxy as a thenable and call
        // a tool named "then"; Symbol keys (`String(symbol)` throws) and `toJSON` likewise.
        "const tools = new Proxy({}, { get(_t, name) { if (typeof name !== 'string' || name === 'then' || name === 'toJSON') return undefined;",
        &format!(" if (name === '{SEARCH_CALL}') return (query) => __tool('{SEARCH_CALL}', {{ query: __fmt(query === undefined ? '' : query) }});"),
        " return (args) => __tool(name, args === undefined ? {} : args); } });",
        "try { const __value = await (async () => {",
    ]
    .concat()
}

const EPILOGUE: &str = "})();\n\
return { ok: true, value: __value === undefined ? null : __value, logs: __logs, dropped: __dropped };\n\
} catch (e) {\n\
let message; try { message = e instanceof Error ? (e.name && e.name !== 'Error' ? e.name + ': ' : '') + e.message : __fmt(e); } catch (_) { message = 'unprintable error'; }\n\
return { ok: false, error: message, logs: __logs, dropped: __dropped };\n\
}\n\
})";

/// Whether an inner call's result should reject in JS: the tool did not run (denied, blocked,
/// refused by a guard, or an `error:`). A shell command that ran and exited non-zero resolves,
/// so a script can read the failing test output without a try/catch.
pub(crate) fn throws(result: &str) -> bool {
    let lower = result.trim_start().to_ascii_lowercase();
    [
        "error:",
        "permission denied",
        "blocked by hook",
        "doom-loop:",
        "stuck:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// An inner call's value as the script sees it: the result plus the hints that call queued (LSP
/// diagnostics after a write, the shell-error diagnosis, loop-guard nudges), so a program that
/// edits and then tests sees its own compile errors.
pub(crate) fn with_hints(result: &str, hints: &[String]) -> String {
    if hints.is_empty() {
        return result.to_string();
    }
    let mut out = result.to_string();
    for hint in hints {
        out.push_str("\n\n[hint] ");
        out.push_str(hint);
    }
    out
}

/// One inner call, for the footer and the failure hint.
#[derive(Debug, Clone)]
pub(crate) struct InnerCall {
    pub(crate) name: String,
    pub(crate) ok: bool,
}

/// How a script ended.
#[derive(Debug)]
pub(crate) enum ScriptEnd {
    Returned(serde_json::Value),
    Threw(String),
    /// The engine refused the script (syntax error, sync-slice interrupt, memory cap, a value
    /// that could not cross the boundary).
    Failed(String),
    /// The deadline passed and the script did not unwind within the grace period.
    Expired,
}

/// Everything the renderer needs from one run.
#[derive(Debug)]
pub(crate) struct ScriptReport {
    pub(crate) end: ScriptEnd,
    pub(crate) logs: Vec<String>,
    pub(crate) dropped_logs: u64,
    pub(crate) calls: Vec<InnerCall>,
    pub(crate) deadline_hit: bool,
    pub(crate) timeout_secs: u64,
    pub(crate) elapsed: std::time::Duration,
}

impl ScriptReport {
    pub(crate) fn succeeded(&self) -> bool {
        matches!(self.end, ScriptEnd::Returned(_))
    }
}

/// Split the epilogue's `{ok, value|error, logs, dropped}` object into an end state and logs.
pub(crate) fn parse_script_value(value: serde_json::Value) -> (ScriptEnd, Vec<String>, u64) {
    let logs = value
        .get("logs")
        .and_then(serde_json::Value::as_array)
        .map(|lines| {
            lines
                .iter()
                .map(|l| l.as_str().map_or_else(|| l.to_string(), str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let dropped = value
        .get("dropped")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0) as u64;
    let end = if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        ScriptEnd::Returned(value.get("value").cloned().unwrap_or_default())
    } else {
        ScriptEnd::Threw(
            value
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("script error")
                .to_string(),
        )
    };
    (end, logs, dropped)
}

/// The model-visible result: the value (or error), the kept logs, and a one-line footer, capped
/// at `max_chars` with the footer always kept.
pub(crate) fn render(report: &ScriptReport, max_chars: usize) -> String {
    let mut body = match &report.end {
        ScriptEnd::Returned(serde_json::Value::String(s)) => s.clone(),
        ScriptEnd::Returned(serde_json::Value::Null) => "(the script returned nothing)".to_string(),
        ScriptEnd::Returned(v) => serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
        ScriptEnd::Threw(msg) | ScriptEnd::Failed(msg) => format!("error: {msg}"),
        ScriptEnd::Expired => format!(
            "error: run_code deadline exceeded after {}s; the script was stopped",
            report.timeout_secs
        ),
    };
    if report.deadline_hit && !matches!(report.end, ScriptEnd::Expired) {
        body.push_str(&format!(
            "\n\nnote: the {}s deadline passed; tool calls after it were refused",
            report.timeout_secs
        ));
    }
    if !report.logs.is_empty() {
        body.push_str("\n\n[logs]\n");
        body.push_str(&report.logs.join("\n"));
        if report.dropped_logs > 0 {
            body.push_str(&format!(
                "\n… {} more log lines dropped",
                report.dropped_logs
            ));
        }
    }
    if !report.succeeded() {
        body.push_str(&failure_hint(report));
    }
    let footer = footer(report);
    let room = max_chars
        .saturating_sub(footer.chars().count() + 2)
        .max(200);
    if body.chars().count() > room {
        let kept: String = body.chars().take(room).collect();
        body = format!("{kept}\n… output truncated at {max_chars} chars; return less");
    }
    format!("{body}\n\n{footer}")
}

/// A rendered result without its trailing footer line, for loop-progress fingerprints.
pub(crate) fn without_footer(result: &str) -> &str {
    match result.trim_end().rsplit_once('\n') {
        Some((body, last)) if last.starts_with("[run_code: ") => body.trim_end(),
        None if result.trim_start().starts_with("[run_code: ") => "",
        _ => result,
    }
}

fn footer(report: &ScriptReport) -> String {
    let ok = report.calls.iter().filter(|c| c.ok).count();
    format!(
        "[run_code: {} tool call{}, {ok} ok, {} failed, {:.1}s]",
        report.calls.len(),
        if report.calls.len() == 1 { "" } else { "s" },
        report.calls.len() - ok,
        report.elapsed.as_secs_f64()
    )
}

fn failure_hint(report: &ScriptReport) -> String {
    let mut hint = String::new();
    if !report.calls.is_empty() {
        let ran: Vec<String> = report
            .calls
            .iter()
            .map(|c| format!("{}{}", c.name, if c.ok { "" } else { " (failed)" }))
            .collect();
        hint.push_str(&format!(
            "\nhint: these calls already ran and keep their effects (writes are snapshotted for \
             /undo): {}",
            ran.join(", ")
        ));
    }
    let message = match &report.end {
        ScriptEnd::Threw(m) | ScriptEnd::Failed(m) => m.as_str(),
        _ => "",
    };
    if message.contains("is not defined") || message.contains("not a function") {
        hint.push_str(
            "\nhint: inside run_code the only globals are `tools` (`await tools.<name>({...})`, \
             `await tools.$search(\"keywords\")`), `console.log` and `log`; there is no \
             fs/fetch/require/setTimeout.",
        );
    }
    hint
}

/// An engine error with its `eval_script:<line>:<col>` positions rewritten to the model's own
/// line numbers (the prelude is line 1; the engine's columns are unreliable, so they go).
pub(crate) fn engine_error(message: &str) -> String {
    static POS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"eval_script:(\d+)(?::\d+)?").expect("static regex")
    });
    POS.replace_all(message, |caps: &regex::Captures<'_>| {
        let line: usize = caps[1].parse().unwrap_or(1);
        format!("line {}", line.saturating_sub(1).max(1))
    })
    .into_owned()
}

#[cfg(test)]
mod tests;
