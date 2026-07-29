//! Direct-provider completeness-audit policy.
//!
//! This module keeps identifier-migration characterization and bounded re-drive guidance together.

use forge_types::{Message, Role};

use crate::is_test_path;

/// Injected for the self-review pass (mesh.self_review): the same model critically re-checks the
/// edits it just made before the turn ends. Framed to FIND real defects (the common failure is a
/// fix that's plausible but wrong/incomplete), but to stop cleanly when the work is sound — so it
/// corrects hard cases without churning correct ones.
pub(crate) const SELF_REVIEW_PROMPT: &str = "\
Before finishing, review the changes you just made as a skeptical senior engineer seeing them for \
the first time. Re-read the original task, then check your diff against it:
- Does it actually solve the stated problem — the whole problem, not just the happy path?
- Edge cases, error handling, off-by-one, wrong/edge inputs, and any case the task hints at.
- Did you edit the right place, match existing conventions, and avoid breaking nearby behavior?
- Is anything missing (a needed call site, a test, a related code path)?

If you find a genuine problem, FIX it now with the tools. If the change is correct and complete, \
say so in one line and stop — do NOT make changes for their own sake or second-guess a sound fix.";

/// Direct-provider counterpart of the bridge completeness re-drive. Unlike the broad same-model
/// self-review (which regressed when always-on), this requires a small, concrete repository-search
/// sweep and permits another edit only when that evidence exposes an omitted production path.
pub(crate) const DIRECT_COMPLETENESS_PROMPT: &str = "\
Before finishing this identifier-migration code-change task, perform ONE bounded completeness \
audit grounded in evidence:
1. Run `git diff` once to inspect the COMPLETE current change, then re-read the original task.
2. You MUST run a targeted repository search sweep for related production paths (maximum THREE \
search commands). If the task or removed diff lines rename, replace, deprecate, or change an \
identifier, option, API, format, or behavior, search for EACH old/deprecated name as a plain \
literal, separately or in a simple alternation. Do NOT add surrounding-syntax predicates such as \
a particular variable, container, call shape, or accessor: those can hide a sibling implementation \
that uses the same name differently. Search the nearest production subsystem containing the edited \
file AND its sibling files; if that finds matches only in files already edited, widen one directory \
level once. Otherwise search for the directly affected callers/implementations.
3. Search-result snippets are NOT inspection. You MUST open the relevant surrounding code in every \
unedited production file returned with an old/deprecated name, especially sibling clients, \
adapters, serializers, commands, and alternate entry points. Explicitly classify whether each path \
implements the same behavior. For a deprecation or rename, any public config/parser/CLI path in the \
affected subsystem that still consumes the old name is a CONCRETE OMISSION until it prefers the \
replacement while retaining the old alias for compatibility (unless the task requires removal). A \
different call shape or downstream consumer does not exempt that path. Do not finish with such an \
unhandled occurrence. Passing existing tests alone is not proof: hidden tests often exercise an \
omitted sibling path.
4. If one is missing, fix that specific omission and run focused verification. If the sweep reveals \
no concrete omission, make NO further edits and finish.

Do not broadly re-explore or refactor, and do not modify existing tests to hide a failure. This is \
one evidence check, not an invitation to second-guess a complete fix.";

/// A direct audit that reports no matches after identifiers disappeared from the diff has not
/// produced trustworthy evidence: path + glob duplication and over-narrow scopes are common model
/// mistakes. Give that case one final, mechanically different search attempt rather than accepting
/// an empty result as proof of completeness.
pub(crate) const DIRECT_COMPLETENESS_EMPTY_SEARCH_RETRY_PROMPT: &str = "\
Your required completeness search reported NO MATCHES even though the task/diff removed or replaced \
old terms. Treat that as a likely search-scope or glob failure, NOT proof that no sibling path exists.
Run ONE final fallback from the repository root with a plain-literal shell search such as \
`git grep -n -F -e '<old-name>' -- <nearest-production-directory>` (add another `-e` for another \
old name). Scope with a directory path OR a file glob, never duplicate the repo-relative prefix in \
both. If it finds an old/deprecated term in an unedited production sibling, OPEN that code and \
handle the concrete omission under the prior compatibility rule. If the fallback truly finds no \
related production occurrence, make no edit and finish. Do not modify tests.";

pub(crate) const DIRECT_COMPLETENESS_MISSING_SEARCH_RETRY_PROMPT: &str = "\
You skipped the REQUIRED repository search in the completeness audit. Do not finish yet. Run ONE \
plain-literal search from the repository root for the old/deprecated identifier(s), using a command \
such as `git grep -n -F -e '<old-name>' -- <nearest-production-directory>`. If it finds an \
old/deprecated term in an unedited production sibling, OPEN that code and handle the concrete \
omission under the prior compatibility rule. Do not modify tests.";

pub(crate) const DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT: &str = "\
Your completeness evidence identified the related production path(s) listed below, but the final \
diff still leaves them unchanged. That is unresolved evidence, not a completed audit. Open each \
listed path and address the task there NOW. For a deprecation/rename, make the replacement name \
preferred while preserving the old name only as a compatibility fallback. Do not run more broad \
searches. Leaving a listed path unchanged is allowed ONLY when its current text contains none of \
the old/deprecated identifiers; a different consumer or call shape is not an exemption. If it \
reads an old identifier from configuration/options, edit it to prefer the replacement. Do not edit \
tests, and do not finish with prose that merely repeats the changes already made.";

pub(crate) const DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT: &str = "\
The mandatory reconciliation ended without changing the evidence-backed production path(s) listed \
below. Final prose cannot override repository state. Re-open each path and make the concrete edit \
NOW. For a deprecation/rename, any path that reads an old identifier from configuration/options \
must prefer the replacement and keep the old name only as fallback compatibility; a different \
consumer or call shape is not an exemption. Inspect the final diff and run focused verification. \
Do not edit tests or repeat the prior summary without acting.";

pub(crate) const DIRECT_NAMED_API_SCOPE_GUIDANCE: &str = "\
The task explicitly names the production API(s) below. Treat those API boundaries as the default \
implementation scope. Before changing a shared lower-level helper instead, OPEN its production \
callers and establish that behavior remains unchanged outside the requested API and when the new \
option or condition is absent/default. Prefer the smallest conditional change at the named API \
unless concrete caller evidence requires the shared helper. Preserve the named API's existing \
control-flow guards, indentation, post-processing, squeeze/shape behavior, and return paths outside \
that smallest conditional. After editing, inspect the complete function and diff for any moved or \
deleted guard.";

/// First-pass scope guard for direct-provider identifier migrations. This is injected into the
/// initial context pack, so it improves discovery without paying for a second provider round.
pub(crate) const DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE: &str = "\
This task migrates a deprecated/renamed identifier, option, or API. Before editing, do ONE bounded \
production-scope sweep:
- Search for EACH old/deprecated identifier as a plain literal in the nearest production subsystem \
containing the named code (maximum TWO search commands). Do not search tests alone, and do not add \
surrounding-syntax predicates that can hide a sibling implementation.
- Open every unedited production sibling match that may implement the same behavior, especially \
clients, adapters, serializers, commands, parsers, and alternate entry points. Search snippets are \
not inspection.
- Update every concrete same-behavior path. For a deprecation/rename, any config/parser/CLI path in \
the affected subsystem that still consumes the old name is a CONCRETE OMISSION until it prefers \
the replacement while retaining the old alias as a fallback unless removal is explicit. A \
different call shape or downstream consumer does not exempt that path.

Keep the sweep bounded; do not broadly review or refactor unrelated code.";

pub(crate) fn completeness_search_reported_no_matches(messages: &[Message]) -> bool {
    messages.iter().any(|message| {
        if message.role != Role::Tool {
            return false;
        }
        let content = message.content.to_ascii_lowercase();
        content.contains("no matches for") || content.contains("0 matches for")
    })
}

pub(crate) fn completeness_repository_search_ran(messages: &[Message]) -> bool {
    messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .any(|call| {
            if call.name == "search" {
                return true;
            }
            if call.name != "shell" {
                return false;
            }
            call.args
                .get("command")
                .and_then(|command| command.as_str())
                .map(|command| {
                    let command = command.to_ascii_lowercase();
                    command.contains("git grep")
                        || command.contains("rg ")
                        || command.contains("grep ")
                })
                .unwrap_or(false)
        })
}

/// Production files returned by the primary solve's required plain-literal identifier search.
/// Test-only, unrelated, empty, and unparseable searches produce no evidence, keeping the
/// turn-end completeness fallback enabled.
pub(crate) fn completeness_production_identifier_search_matches(
    messages: &[Message],
    workspace_root: &std::path::Path,
    prompt: &str,
) -> std::collections::BTreeSet<String> {
    let prompt_terms = prompt
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|term| term.len() >= 3)
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::HashSet<_>>();

    let mut matches = std::collections::BTreeSet::new();
    for call in messages
        .iter()
        .flat_map(|message| &message.tool_calls)
        .filter(|call| call.name == "search")
    {
        let Some(raw_path) = call.args.get("path").and_then(|path| path.as_str()) else {
            continue;
        };
        let path = std::path::Path::new(raw_path);
        let relative_search_root = if path.is_absolute() {
            match path.strip_prefix(workspace_root) {
                Ok(relative) => relative,
                Err(_) => continue,
            }
        } else {
            path
        };
        let relative_search_root = relative_search_root
            .to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches("./")
            .to_string();
        if is_test_path(&relative_search_root) {
            continue;
        }

        let query_mentions_prompt_identifier = call
            .args
            .get("query")
            .and_then(|query| query.as_str())
            .into_iter()
            .flat_map(|query| query.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_')))
            .filter(|term| term.len() >= 3)
            .map(str::to_ascii_lowercase)
            .any(|term| prompt_terms.contains(&term));
        if !query_mentions_prompt_identifier {
            continue;
        }

        let Some(result) = messages.iter().find(|message| {
            message.role == Role::Tool && message.tool_call_id.as_deref() == Some(call.id.as_str())
        }) else {
            continue;
        };
        for line in result.content.lines() {
            let Some((raw_match_path, _)) = line.split_once(':') else {
                continue;
            };
            let raw_match_path = raw_match_path.trim();
            if raw_match_path.is_empty() {
                continue;
            }
            let match_path = std::path::Path::new(raw_match_path);
            let relative_match = if match_path.is_absolute() {
                match match_path.strip_prefix(workspace_root) {
                    Ok(relative) => relative.to_path_buf(),
                    Err(_) => continue,
                }
            } else if relative_search_root.is_empty()
                || match_path.starts_with(std::path::Path::new(&relative_search_root))
            {
                match_path.to_path_buf()
            } else {
                std::path::Path::new(&relative_search_root).join(match_path)
            };
            let relative_match = relative_match
                .to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches("./")
                .to_string();
            if !relative_match.is_empty() && !is_test_path(&relative_match) {
                matches.insert(relative_match);
            }
        }
    }
    matches
}

pub(crate) fn direct_completeness_is_identifier_migration(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    [
        "deprecat",
        "renam",
        "old name",
        "new name",
        "alias",
        "backward compat",
        "backwards compat",
        "compatibility fallback",
    ]
    .iter()
    .any(|signal| prompt.contains(signal))
}

pub(crate) fn direct_scope_guidance_named_apis(prompt: &str) -> Vec<String> {
    let mut names = prompt
        .split_whitespace()
        .filter_map(|token| {
            let token = token
                .trim_matches(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.'));
            let token = token.trim_matches('.');
            let (owner, member) = token.split_once('.')?;
            if owner.contains('.') || member.contains('.') {
                return None;
            }
            let mut owner_chars = owner.chars();
            let first = owner_chars.next()?;
            let second = owner_chars.next()?;
            let owner_is_class = first.is_ascii_uppercase()
                && second.is_ascii_lowercase()
                && owner
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            let member_is_method = member
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_')
                && member
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
            (owner_is_class && member_is_method).then(|| token.to_string())
        })
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

pub(crate) fn changed_paths_from_status(
    status: Option<&[u8]>,
) -> std::collections::HashSet<String> {
    status
        .map(String::from_utf8_lossy)
        .into_iter()
        .flat_map(|status| {
            status
                .lines()
                .filter_map(|line| {
                    if line.len() < 4 {
                        return None;
                    }
                    let raw_path = &line[3..];
                    let path = raw_path
                        .rsplit_once(" -> ")
                        .map(|(_, destination)| destination)
                        .unwrap_or(raw_path)
                        .trim()
                        .trim_matches('"')
                        .replace('\\', "/");
                    (!path.is_empty()).then_some(path)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

pub(crate) fn opened_unchanged_production_paths(
    messages: &[Message],
    workspace_root: &std::path::Path,
    changed_paths: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut paths = std::collections::BTreeSet::new();
    for call in messages.iter().flat_map(|message| &message.tool_calls) {
        if call.name != "read_file" {
            continue;
        }
        let Some(raw_path) = forge_types::extract_path_arg(&call.args) else {
            continue;
        };
        let path = std::path::Path::new(raw_path);
        let relative = if path.is_absolute() {
            match path.strip_prefix(workspace_root) {
                Ok(relative) => relative,
                Err(_) => continue,
            }
        } else {
            path
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if !relative.is_empty() && !is_test_path(&relative) && !changed_paths.contains(&relative) {
            paths.insert(relative);
        }
    }
    paths.into_iter().collect()
}

pub(crate) fn unresolved_completeness_production_paths(
    primary_opened_identifier_paths: &std::collections::BTreeSet<String>,
    audit_messages: &[Message],
    workspace_root: &std::path::Path,
    changed_paths: &std::collections::HashSet<String>,
) -> Vec<String> {
    let mut paths =
        opened_unchanged_production_paths(audit_messages, workspace_root, changed_paths)
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
    paths.extend(
        primary_opened_identifier_paths
            .iter()
            .filter(|path| !changed_paths.contains(path.as_str()))
            .cloned(),
    );
    paths.into_iter().collect()
}

/// Whether a `shell` tool result reports a failure (non-zero exit, signal, timeout, or spawn
/// error). The tool's first line is `shell: exit N in …`, `shell: timed out …`, `shell: error: …`,
/// or `shell: failed to start …`; only `exit 0` is success.
pub(crate) fn shell_command_failed(result: &str) -> bool {
    let first = result.lines().next().unwrap_or("");
    match first.strip_prefix("shell: exit ") {
        Some(rest) => {
            rest.split_whitespace()
                .next()
                .and_then(|t| t.parse::<i32>().ok())
                != Some(0)
        }
        None => first.starts_with("shell:"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ErrorCategory {
    Permission,
    NotFound,
    Schema,
    Timeout,
    Other,
}

impl ErrorCategory {
    pub(crate) fn classify(err: &str) -> Self {
        let e = err.to_lowercase();
        if e.contains("permission") || e.contains("denied") || e.contains("forbidden") {
            Self::Permission
        } else if e.contains("not found") || e.contains("no such file") || e.contains("enoent") {
            Self::NotFound
        } else if e.contains("schema") || e.contains("invalid") || e.contains("parse") {
            Self::Schema
        } else if e.contains("timeout") || e.contains("timed out") {
            Self::Timeout
        } else {
            Self::Other
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::NotFound => "not_found",
            Self::Schema => "schema",
            Self::Timeout => "timeout",
            Self::Other => "other",
        }
    }
}

#[derive(Debug)]
pub(crate) struct ToolFailureTracker {
    /// (tool_name, error_category) -> consecutive failure count this turn.
    failure_counts: std::collections::HashMap<(String, ErrorCategory), u32>,
    /// Ring buffer of recent (tool_name, args_hash) calls for doom-loop detection.
    recent_calls: std::collections::VecDeque<(String, u64)>,
    failure_threshold: u32,
    doom_loop_threshold: u32,
}

impl Default for ToolFailureTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolFailureTracker {
    pub(crate) fn new() -> Self {
        Self {
            failure_counts: Default::default(),
            recent_calls: std::collections::VecDeque::with_capacity(10),
            failure_threshold: 3,
            doom_loop_threshold: 3,
        }
    }

    pub(crate) fn reset_turn(&mut self) {
        self.failure_counts.clear();
        self.recent_calls.clear();
    }

    pub(crate) fn record_call(&mut self, tool_name: &str, args_json: &str) -> Option<String> {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        args_json.hash(&mut hasher);
        let h = hasher.finish();

        let key = (tool_name.to_string(), h);
        if self.recent_calls.len() >= 10 {
            self.recent_calls.pop_front();
        }
        self.recent_calls.push_back(key.clone());

        let consecutive = self
            .recent_calls
            .iter()
            .rev()
            .take_while(|k| *k == &key)
            .count() as u32;

        (consecutive >= self.doom_loop_threshold).then(|| {
            format!(
                "doom-loop: `{tool_name}` called identically {consecutive} times in a row — nudging model to try a different approach"
            )
        })
    }

    pub(crate) fn record_failure(&mut self, tool_name: &str, error: &str) -> Option<String> {
        let cat = ErrorCategory::classify(error);
        let key = (tool_name.to_string(), cat);
        let count = self.failure_counts.entry(key).or_insert(0);
        *count += 1;
        (*count >= self.failure_threshold).then(|| {
            format!(
                "stuck: `{tool_name}` failed {count} times ({cat:?}) — check permissions/schema before retrying"
            )
        })
    }

    pub(crate) fn record_success(&mut self, tool_name: &str) {
        self.failure_counts.retain(|(name, _), _| name != tool_name);
    }
}

/// Match common, unambiguous failure patterns in the tool output and return a pre-canned
/// diagnosis — skipping the model call entirely (free, instant). Returns `None` when the
/// failure is unusual enough to need the model. Checked case-insensitively on the full result.
pub(crate) fn pattern_diagnose(result: &str) -> Option<&'static str> {
    // The table is ordered most-specific first so a result with multiple signals hits the
    // most actionable match. Each pattern must be unambiguous: "permission denied" alone
    // could be a file *or* a network ACL — but combining with exit codes is overkill here;
    // the worst case is a slightly generic message, which is still free and instant.
    let lower = result.to_lowercase();
    let has = |s: &str| lower.contains(s);
    if has("command not found") || has("no such file or directory") && has("exec") {
        return Some("Command not found — check it is installed and in PATH.");
    }
    if has("no such file or directory") {
        return Some("File or directory does not exist — verify the path with `ls` or `pwd`.");
    }
    if has("permission denied") || has("operation not permitted") {
        return Some("Permission denied — try `chmod +x <file>` or prefix with `sudo`.");
    }
    if has("address already in use") {
        return Some(
            "Port already in use — find the process with `lsof -i :<port>` or `ss -tlnp`.",
        );
    }
    if has("connection refused") {
        return Some("Connection refused — the target service may not be running.");
    }
    if has("no space left on device") || has("disk quota exceeded") {
        return Some("Disk full or quota exceeded — free space with `df -h` and `du -sh *`.");
    }
    if has("out of memory") || has("cannot allocate memory") {
        return Some("Out of memory — reduce concurrency or increase available RAM/swap.");
    }
    None
}

/// Whether `finding_sev` is at or above `threshold` (a string from `AssayConfig::gate_severity`).
/// Ordering (most → least severe): critical > high > medium > low.
/// A "high" threshold matches `high` and `critical` but not `medium` or `low`.
/// Returns `true` for any unrecognised threshold string (fail-open: surface the finding rather than
/// silently drop it when the config has a typo).
pub(crate) fn severity_meets(finding_sev: forge_types::Severity, threshold: &str) -> bool {
    use forge_types::Severity;
    let min_weight = match threshold.trim().to_lowercase().as_str() {
        "critical" => Severity::Critical.weight(),
        "high" => Severity::High.weight(),
        "medium" | "med" => Severity::Medium.weight(),
        "low" => Severity::Low.weight(),
        _ => 0, // unknown threshold → pass everything through
    };
    finding_sev.weight() >= min_weight
}

/// Adopt a post-turn re-drive's answer into the turn's `final_text` — but ONLY when the re-drive
/// actually produced one.
///
/// A re-drive (autofix, empty-diff nudge, test-edit guard, stop-hook continuation) re-enters
/// [`Session::run_model_loop`], whose `final_text` starts empty and STAYS empty whenever that inner
/// loop ends via a repetition/failure guard halt, the empty-response dead-end, or the step cap.
/// Assigning it unconditionally destroyed the primary turn's real answer: Forge emitted
/// `Done { final_text: "" }` and returned an empty `LoopOutcome`, so `forge_chat` handed its caller
/// an empty response and the TUI's final answer block was blank — for work that HAD been done.
/// This matches the self-review pass, which deliberately keeps the original answer text.
pub(crate) fn adopt_redrive_text(final_text: &mut String, redrive_text: String) {
    if !redrive_text.trim().is_empty() {
        *final_text = redrive_text;
    }
}
