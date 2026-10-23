//! Completion policy shared by every Forge execution surface.
//!
//! A model saying that work is done is not evidence by itself. This contract keeps the
//! verification rule independent from the direct-provider and CLI-bridge loops so they cannot
//! silently accept different definitions of completion.

use crate::TaskIntent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum VerificationFamily {
    Typecheck,
    Lint,
    Test,
    Build,
}

impl VerificationFamily {
    const fn label(self) -> &'static str {
        match self {
            Self::Typecheck => "typecheck",
            Self::Lint => "lint",
            Self::Test => "test",
            Self::Build => "build",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerificationObservation {
    Ignore,
    Mutation,
    Generic,
    Check(VerificationFamily),
}

/// Outcome-aware evidence for the completion gate. Failed verification families remain unresolved
/// until that same family succeeds; unrelated reads can add evidence but cannot erase a failure.
#[derive(Debug, Default)]
pub(crate) struct VerificationLedger {
    unresolved: std::collections::BTreeSet<VerificationFamily>,
    successful_observations: u64,
    /// Observation counter at the most recent successful artifact mutation. Evidence at or before
    /// this point is stale: a write after a passing check must be followed by another check.
    last_mutation_checkpoint: u64,
    /// A test, lint, typecheck or build passed after the latest successful artifact edit. Stricter
    /// than [`Self::verified_since`], which any successful read satisfies.
    checked_since_edit: bool,
    /// A file tool (not a shell command) changed an artifact this turn. Untracked files that
    /// appeared without one are what a command left behind (a server's state directory, a
    /// build), not the model's authorship.
    authored: bool,
}

impl VerificationLedger {
    pub(crate) const fn checkpoint(&self) -> u64 {
        self.successful_observations
    }

    pub(crate) fn observe(&mut self, observation: VerificationObservation, ok: bool) {
        match observation {
            VerificationObservation::Ignore => {}
            VerificationObservation::Mutation => {
                if ok {
                    self.last_mutation_checkpoint = self.successful_observations;
                    self.checked_since_edit = false;
                    self.authored = true;
                }
            }
            VerificationObservation::Generic => {
                if ok {
                    self.successful_observations = self.successful_observations.saturating_add(1);
                }
            }
            VerificationObservation::Check(family) => {
                if ok {
                    self.unresolved.remove(&family);
                    self.successful_observations = self.successful_observations.saturating_add(1);
                    self.checked_since_edit = true;
                } else {
                    self.unresolved.insert(family);
                }
            }
        }
    }

    pub(crate) fn verified_since(&self, checkpoint: u64) -> bool {
        self.unresolved.is_empty()
            && self.successful_observations > checkpoint.max(self.last_mutation_checkpoint)
    }

    /// A real check (not just a read) passed after the latest edit and none is still failing.
    pub(crate) fn checked_since_edit(&self) -> bool {
        self.checked_since_edit && self.unresolved.is_empty()
    }

    /// A file tool changed an artifact this turn.
    pub(crate) const fn authored(&self) -> bool {
        self.authored
    }

    /// A check family failed and has not passed since.
    pub(crate) fn has_failing_check(&self) -> bool {
        !self.unresolved.is_empty()
    }

    pub(crate) fn unresolved_summary(&self) -> Option<String> {
        (!self.unresolved.is_empty()).then(|| {
            self.unresolved
                .iter()
                .map(|family| family.label())
                .collect::<Vec<_>>()
                .join(", ")
        })
    }
}

/// Whether a tool NAME denotes a state-changing call (a write-family tool or a shell). Used where
/// only the name is available: a CLI bridge runs its tool loop in a subprocess and surfaces calls
/// as stream events, so there is no local registry to ask for a [`forge_types::SideEffect`]. The
/// direct path uses the registry instead — see `Session::execute_model_tool_step`. Suffix matching
/// keeps it working for the bridge's namespaced names (`mcp__forge__write_file`).
pub(crate) fn tool_name_mutates(name: &str) -> bool {
    [
        "write_file",
        "append_file",
        "edit_file",
        "multi_edit",
        "apply_patch",
        "delete_file",
        "move_file",
        "copy_file",
        "notebook_edit",
        "shell",
        "exec_command",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
}

pub(crate) fn classify_tool(name: &str, args: &str) -> VerificationObservation {
    if [
        "update_tasks",
        "present_plan",
        "use_skill",
        "spawn_agents",
        "ask_user",
        // Its inner calls are observed one by one; the wrapper itself is not evidence.
        "run_code",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
    {
        return VerificationObservation::Ignore;
    }
    if [
        "write_file",
        "append_file",
        "edit_file",
        "multi_edit",
        "notebook_edit",
        "apply_patch",
        "delete_file",
        "move_file",
        "copy_file",
    ]
    .iter()
    .any(|suffix| name.ends_with(suffix))
    {
        // Sweeping up what a test run left (`__pycache__/`) is not an edit to the work: counting it
        // as one made every passing check stale, and the model re-verified to clean up again.
        return if touches_only_run_artifacts(args) {
            VerificationObservation::Ignore
        } else {
            VerificationObservation::Mutation
        };
    }
    if !name.ends_with("shell") && !name.ends_with("exec_command") {
        return VerificationObservation::Generic;
    }
    let value = serde_json::from_str::<serde_json::Value>(args).ok();
    // A `commands` batch runs each entry as its own process, so one entry's pipe or `;` cannot mask
    // another's exit status, and the batch reports the first failure. Joined into one script it
    // always looked like a multi-line command with unchecked status, and no batched test run ever
    // counted as a check.
    if let Some(entries) = value.as_ref().and_then(shell_batch_entries) {
        let mut check = None;
        let mut masked = false;
        for entry in entries {
            match classify_command(entry) {
                VerificationObservation::Check(family) => check = Some(family),
                VerificationObservation::Ignore => masked = true,
                _ => {}
            }
        }
        return match (check, masked) {
            (Some(family), _) => VerificationObservation::Check(family),
            (None, true) => VerificationObservation::Ignore,
            (None, false) => VerificationObservation::Generic,
        };
    }
    let command = value
        .as_ref()
        .and_then(forge_types::shell_command_text)
        .unwrap_or_else(|| args.to_owned());
    classify_command(&command)
}

/// The entries of a `shell` call's `commands` batch, when it is one.
fn shell_batch_entries(args: &serde_json::Value) -> Option<Vec<&str>> {
    if args.get("command").is_some() {
        return None;
    }
    let entries: Vec<&str> = args
        .get("commands")?
        .as_array()?
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    (!entries.is_empty()).then_some(entries)
}

/// Every path a file tool's arguments name is a build or test-run byproduct, and there is one.
fn touches_only_run_artifacts(args: &str) -> bool {
    fn collect<'a>(value: &'a serde_json::Value, out: &mut Vec<&'a str>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    match inner {
                        serde_json::Value::String(path)
                            if matches!(key.as_str(), "path" | "file_path" | "from" | "to") =>
                        {
                            out.push(path)
                        }
                        serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
                            collect(inner, out)
                        }
                        _ => {}
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    match item {
                        serde_json::Value::String(path) => out.push(path),
                        other => collect(other, out),
                    }
                }
            }
            _ => {}
        }
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(args) else {
        return false;
    };
    let mut paths = Vec::new();
    collect(&value, &mut paths);
    !paths.is_empty()
        && paths
            .iter()
            .all(|path| crate::working_tree::is_run_artifact(path))
}

/// One shell command line: which family of check it is, and whether its exit status can be
/// trusted as that check's result.
fn classify_command(command: &str) -> VerificationObservation {
    let command = command.to_ascii_lowercase();
    let family = if command.contains("tsc")
        || command.contains("typecheck")
        || command.contains("type-check")
        || command.contains("cargo check")
        || command.contains("node --check")
        || command.contains("vm.script")
        || command.contains("syntax")
    {
        Some(VerificationFamily::Typecheck)
    } else if command.contains("eslint")
        || command.contains("clippy")
        || command.contains(" lint")
        || command.contains("lint ")
    {
        Some(VerificationFamily::Lint)
    } else if command.contains("test")
        || command.contains("pytest")
        || command.contains("vitest")
        || command.contains("jest")
        || command.contains("nextest")
        || command.contains("self-check")
        || command.contains("selfcheck")
    {
        Some(VerificationFamily::Test)
    } else if command.contains("build")
        || command.contains("compile")
        || command.contains("xcodebuild")
    {
        Some(VerificationFamily::Build)
    } else {
        None
    };
    match family {
        // POSIX pipelines report the final stage's status, and `;` / `||` chains can overwrite a
        // failed check with a later successful command. Never accept masked status as proof.
        Some(_) if has_untrustworthy_control_flow(&command) => VerificationObservation::Ignore,
        Some(family) => VerificationObservation::Check(family),
        None => VerificationObservation::Generic,
    }
}

fn has_untrustworthy_control_flow(command: &str) -> bool {
    let shell = strip_heredoc_bodies(command);
    let operators = strip_quoted_literals(&shell);
    if operators.contains(';')
        || operators.contains("||")
        || has_standalone_background_operator(&operators)
    {
        return true;
    }
    if shell.lines().filter(|line| !line.trim().is_empty()).count() > 1
        && (!starts_with_errexit(&shell) || later_disables_or_masks_errexit(&shell))
    {
        return true;
    }
    let bytes = operators.as_bytes();
    let has_pipeline = bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'|'
            && index.checked_sub(1).and_then(|i| bytes.get(i)) != Some(&b'|')
            && bytes.get(index + 1) != Some(&b'|')
    });
    has_pipeline && !enables_pipefail(&shell)
}

/// Remove heredoc payloads before evaluating shell control flow. Python/Ruby/SQL embedded in a
/// fail-fast shell command can legitimately contain `if`, `while`, pipes, and semicolons; those
/// tokens are data consumed by the heredoc command and cannot mask the shell command's exit code.
fn strip_heredoc_bodies(command: &str) -> String {
    let mut output = String::with_capacity(command.len());
    let mut delimiter: Option<String> = None;
    for line in command.lines() {
        if let Some(expected) = delimiter.as_deref() {
            if line.trim() == expected {
                delimiter = None;
            }
            output.push('\n');
            continue;
        }
        output.push_str(line);
        output.push('\n');
        delimiter = heredoc_delimiter(line);
    }
    output
}

fn heredoc_delimiter(line: &str) -> Option<String> {
    let bytes = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut index = 0;
    while index + 1 < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && quote != Some(b'\'') {
            index = index.saturating_add(2);
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
            index += 1;
            continue;
        }
        if quote.is_none()
            && byte == b'<'
            && bytes.get(index + 1) == Some(&b'<')
            && bytes.get(index + 2) != Some(&b'<')
        {
            index += 2;
            if bytes.get(index) == Some(&b'-') {
                index += 1;
            }
            while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
                index += 1;
            }
            let delimiter_quote = bytes
                .get(index)
                .copied()
                .filter(|byte| matches!(byte, b'\'' | b'"'));
            if delimiter_quote.is_some() {
                index += 1;
            }
            let start = index;
            while let Some(byte) = bytes.get(index) {
                if delimiter_quote == Some(*byte)
                    || (delimiter_quote.is_none()
                        && (byte.is_ascii_whitespace()
                            || matches!(*byte, b';' | b'|' | b'&' | b'<' | b'>')))
                {
                    break;
                }
                index += 1;
            }
            return (index > start).then(|| line[start..index].to_string());
        }
        index += 1;
    }
    None
}

/// Mask single-quoted literal contents before looking for shell operators.
///
/// Double-quoted text is deliberately preserved: command substitutions (`$(...)` and backticks)
/// execute inside double quotes and can mask a failing verification. Treating an inert semicolon
/// in a double-quoted label as suspicious is a safe false negative; accepting a masked check is not.
fn strip_quoted_literals(command: &str) -> String {
    let first_line_words = command
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>();
    let invokes_shell_command_string = first_line_words
        .iter()
        .enumerate()
        .find(|(_, word)| {
            word.rsplit('/')
                .next()
                .is_some_and(|shell| matches!(shell, "bash" | "zsh" | "ksh" | "sh"))
        })
        .is_some_and(|(shell_index, _)| {
            first_line_words[shell_index + 1..].iter().any(|word| {
                word.strip_prefix('-')
                    .is_some_and(|flags| !flags.starts_with('-') && flags.contains('c'))
            })
        });
    if invokes_shell_command_string {
        // Quotes delimit executable shell source after `-c`, not inert data. Preserve its
        // operators so `sh -c 'tests; true'` cannot be accepted as trustworthy evidence.
        return command.to_string();
    }
    let bytes = command.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut quote: Option<u8> = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && quote != Some(b'\'') {
            output.push(byte);
            if let Some(next) = bytes.get(index + 1) {
                output.push(*next);
            }
            index += 2;
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
            output.push(b' ');
        } else {
            output.push(if quote == Some(b'\'') { b' ' } else { byte });
        }
        index += 1;
    }
    String::from_utf8(output).expect("shell command began as UTF-8")
}

fn starts_with_errexit(command: &str) -> bool {
    let Some(first) = command
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
    else {
        return false;
    };
    let words = first.split_whitespace().collect::<Vec<_>>();
    if words.first() != Some(&"set") {
        return false;
    }
    words
        .iter()
        .skip(1)
        .any(|word| word.starts_with('-') && !word.starts_with("--") && word.contains('e'))
        || words.windows(2).any(|pair| pair == ["-o", "errexit"])
}

fn later_disables_or_masks_errexit(command: &str) -> bool {
    command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .skip(1)
        .any(|line| {
            let words = line.split_whitespace().collect::<Vec<_>>();
            let disables_errexit = words.first() == Some(&"set")
                && (words
                    .iter()
                    .skip(1)
                    .any(|word| word.starts_with('+') && word.contains('e'))
                    || words.windows(2).any(|pair| pair == ["+o", "errexit"]));
            let masks_status = matches!(
                words.first().copied(),
                Some("!") | Some("if") | Some("while") | Some("until")
            );
            disables_errexit || masks_status
        })
}

fn enables_pipefail(command: &str) -> bool {
    let mut enabled = false;
    for line in command
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let words = line.split_whitespace().collect::<Vec<_>>();
        if words
            .first()
            .and_then(|word| word.rsplit('/').next())
            .is_some_and(|shell| matches!(shell, "bash" | "zsh" | "ksh" | "sh"))
            && words.windows(2).any(|pair| pair == ["-o", "pipefail"])
        {
            enabled = true;
            continue;
        }
        if words.first() != Some(&"set") {
            continue;
        }
        if words.windows(2).any(|pair| pair == ["+o", "pipefail"]) {
            enabled = false;
            continue;
        }
        if words.windows(2).any(|pair| pair == ["-o", "pipefail"])
            || (words
                .iter()
                .skip(1)
                .any(|word| word.starts_with('-') && word.contains('o'))
                && words.contains(&"pipefail"))
        {
            enabled = true;
        }
    }
    enabled
}

fn has_standalone_background_operator(command: &str) -> bool {
    let bytes = command.as_bytes();
    bytes.iter().enumerate().any(|(index, byte)| {
        if *byte != b'&' {
            return false;
        }
        let previous = index.checked_sub(1).and_then(|i| bytes.get(i));
        let next = bytes.get(index + 1);
        previous != Some(&b'&')
            && next != Some(&b'&')
            && previous != Some(&b'>')
            && previous != Some(&b'<')
            && next != Some(&b'>')
    })
}

/// Evidence observed while a model claims that every tracked task is complete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CompletionEvidence {
    /// The turn performed work that left an external artifact which can be inspected.
    pub(crate) did_real_work: bool,
    /// The model explicitly established that a change was not required.
    pub(crate) no_change_required: bool,
    /// The current turn inspected real state rather than merely repeating its claim.
    pub(crate) inspected_this_turn: bool,
}

/// The action the agent loop takes after an all-done claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionDecision {
    /// Ask for a tool-grounded observation before accepting the claim.
    RequestObservation,
    /// The claim is backed by an inspection.
    AcceptClean,
    /// There was no external artifact to inspect.
    AcceptNoArtifacts,
    /// Verification was requested but never provided before the bounded retry budget expired.
    AcceptUnverified,
}

/// Bounded completion-verification policy for an execution surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CompletionContract {
    max_observation_requests: usize,
}

impl CompletionContract {
    /// Construct a contract with an explicit bounded observation budget (primarily for tests).
    pub(crate) const fn with_observation_budget(max_observation_requests: usize) -> Self {
        Self {
            max_observation_requests,
        }
    }

    /// The production policy allows two observation requests before accepting an explicitly
    /// unverified completion. This preserves Forge's existing anti-spiral behavior.
    pub(crate) const fn production() -> Self {
        Self::with_observation_budget(2)
    }

    pub(crate) const fn max_observation_requests(self) -> usize {
        self.max_observation_requests
    }

    /// Decide whether completion is credible from the observed evidence.
    pub(crate) fn decide(
        self,
        intent: TaskIntent,
        observation_requests: usize,
        evidence: CompletionEvidence,
    ) -> CompletionDecision {
        if intent.is_observational() || evidence.no_change_required {
            return if evidence.inspected_this_turn {
                CompletionDecision::AcceptClean
            } else {
                CompletionDecision::AcceptNoArtifacts
            };
        }

        // A successful inspection that is newer than the last artifact mutation is already the
        // proof we would ask the model to produce. Accept it immediately even when it happened
        // just before `update_tasks` marked the plan Done; bookkeeping does not stale evidence.
        if evidence.inspected_this_turn {
            return CompletionDecision::AcceptClean;
        }

        if observation_requests > 0 && !evidence.did_real_work {
            return CompletionDecision::AcceptNoArtifacts;
        }

        if observation_requests < self.max_observation_requests {
            CompletionDecision::RequestObservation
        } else {
            CompletionDecision::AcceptUnverified
        }
    }
}

/// Whether completion text explicitly states that no external change was needed.
pub(crate) fn claims_no_change(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "no change needed",
        "no changes needed",
        "no change is needed",
        "no changes are needed",
        "no change required",
        "no changes required",
        "make no changes",
        "no file changes",
        "already satisfied",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

/// The bridge sent no assistant text after a verification request, which is terminal only when
/// the prior assistant answer already completed every tracked task.
pub(crate) fn empty_verification_is_terminal(
    observation_requests: usize,
    tasks: &[forge_types::TodoItem],
    has_prior_final: bool,
) -> bool {
    observation_requests > 0
        && !tasks.is_empty()
        && tasks
            .iter()
            .all(|task| matches!(task.status, forge_types::TodoStatus::Done))
        && has_prior_final
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_claims_are_challenged_then_evaluated_from_evidence() {
        let contract = CompletionContract::production();
        let work = CompletionEvidence {
            did_real_work: true,
            ..CompletionEvidence::default()
        };
        assert_eq!(
            contract.decide(TaskIntent::Mutating, 0, work),
            CompletionDecision::RequestObservation
        );
        assert_eq!(
            contract.decide(TaskIntent::Mutating, 2, work),
            CompletionDecision::AcceptUnverified
        );
        assert_eq!(
            contract.decide(
                TaskIntent::Mutating,
                0,
                CompletionEvidence {
                    inspected_this_turn: true,
                    ..work
                }
            ),
            CompletionDecision::AcceptClean
        );
    }

    #[test]
    fn a_batched_test_run_is_a_check_like_the_same_commands_called_one_by_one() {
        // Forge's own `shell` tool takes `commands`; each entry is its own process, so the
        // entries cannot mask one another's status the way lines of one script can.
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"commands":["cargo build","cargo test -p forge-core"]}"#
            ),
            VerificationObservation::Check(VerificationFamily::Test)
        );
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"commands":["python3 -m pytest tests/test_scheduler.py","git diff --check && git diff"]}"#
            ),
            VerificationObservation::Check(VerificationFamily::Test)
        );
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"commands":["git status --short","git diff HEAD"]}"#
            ),
            VerificationObservation::Generic
        );
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"commands":["pytest | tail -5","git status --short"]}"#
            ),
            VerificationObservation::Ignore,
            "an entry whose own status is masked by a pipe proves nothing"
        );
        assert_eq!(
            classify_tool("shell", "{\"command\":\"cargo test\\ngit status\"}"),
            VerificationObservation::Ignore,
            "lines of one script still share a status"
        );
    }

    #[test]
    fn sweeping_up_test_run_byproducts_does_not_stale_a_passing_check() {
        let mut ledger = VerificationLedger::default();
        ledger.observe(classify_tool("edit_file", r#"{"path":"src/a.py"}"#), true);
        assert!(ledger.authored());
        ledger.observe(classify_tool("shell", r#"{"command":"pytest"}"#), true);
        assert!(ledger.checked_since_edit());
        for cleanup in [
            r#"{"path":"src/__pycache__/a.cpython-312.pyc"}"#,
            r#"{"paths":["tests/__pycache__/t.pyc",".pytest_cache/x"]}"#,
        ] {
            assert_eq!(
                classify_tool("delete_file", cleanup),
                VerificationObservation::Ignore,
                "{cleanup}"
            );
            ledger.observe(classify_tool("delete_file", cleanup), true);
        }
        assert!(ledger.checked_since_edit(), "the check still stands");
        assert_eq!(
            classify_tool("delete_file", r#"{"path":"src/a.py"}"#),
            VerificationObservation::Mutation
        );
        assert_eq!(
            classify_tool(
                "multi_edit",
                r#"{"edits":[{"path":"src/__pycache__/a.pyc"},{"path":"src/a.py"}]}"#
            ),
            VerificationObservation::Mutation,
            "one real path makes it an edit"
        );
    }

    #[test]
    fn a_shell_only_turn_authored_nothing() {
        let mut ledger = VerificationLedger::default();
        ledger.observe(classify_tool("shell", r#"{"command":"./serve.sh"}"#), true);
        assert!(!ledger.authored());
        ledger.observe(classify_tool("write_file", r#"{"path":"notes.md"}"#), false);
        assert!(!ledger.authored(), "a failed write changed nothing");
        ledger.observe(classify_tool("write_file", r#"{"path":"notes.md"}"#), true);
        assert!(ledger.authored());
    }

    #[test]
    fn artifact_mutation_stales_old_evidence_until_a_new_check_passes() {
        let mut ledger = VerificationLedger::default();
        ledger.observe(classify_tool("shell", r#"{"command":"npm test"}"#), true);
        assert!(ledger.verified_since(0));

        ledger.observe(
            classify_tool("append_file", r#"{"path":"index.html","content":"x"}"#),
            true,
        );
        assert!(
            !ledger.verified_since(0),
            "a write after the test must stale that test result"
        );

        ledger.observe(
            classify_tool("shell", r#"{"command":"node --check extracted-script.js"}"#),
            true,
        );
        assert!(ledger.verified_since(0));
    }

    #[test]
    fn a_passing_check_after_the_last_edit_is_tracked_apart_from_plain_reads() {
        let check = classify_tool("shell", r#"{"command":"cargo test"}"#);
        let read = classify_tool("read_file", "{}");
        let mut ledger = VerificationLedger::default();
        assert!(!ledger.checked_since_edit());

        ledger.observe(VerificationObservation::Mutation, true);
        ledger.observe(read, true);
        assert!(
            ledger.verified_since(0),
            "a read is evidence for the completion gate"
        );
        assert!(!ledger.checked_since_edit(), "but it is not a check");

        ledger.observe(check, true);
        assert!(ledger.checked_since_edit());
        ledger.observe(VerificationObservation::Mutation, true);
        assert!(
            !ledger.checked_since_edit(),
            "an edit makes the last check stale"
        );

        ledger.observe(check, false);
        assert!(ledger.has_failing_check());
        ledger.observe(check, true);
        assert!(!ledger.has_failing_check());
        assert!(ledger.checked_since_edit());
    }

    #[test]
    fn writes_and_orchestration_are_not_mistaken_for_inspection() {
        for tool in [
            "write_file",
            "append_file",
            "mcp__forge__edit_file",
            "apply_patch",
        ] {
            assert_eq!(
                classify_tool(tool, "{}"),
                VerificationObservation::Mutation,
                "{tool}"
            );
        }
        for tool in ["use_skill", "spawn_agents", "update_tasks", "present_plan"] {
            assert_eq!(
                classify_tool(tool, "{}"),
                VerificationObservation::Ignore,
                "{tool}"
            );
        }
    }

    #[test]
    fn javascript_syntax_and_embedded_self_checks_are_verification_families() {
        assert_eq!(
            classify_tool("shell", r#"{"command":"node --check app.js"}"#),
            VerificationObservation::Check(VerificationFamily::Typecheck)
        );
        assert_eq!(
            classify_tool("shell", r#"{"command":"node -e 'window.runSelfCheck()'"}"#,),
            VerificationObservation::Check(VerificationFamily::Test)
        );
    }

    #[test]
    fn piped_checks_are_not_trusted_without_failure_propagation() {
        for command in [
            "cargo test 2>&1 | tail -50",
            "npm test | tee test.log",
            "cargo clippy | head -40",
            "tsc --noEmit | sed -n '1,80p'",
        ] {
            assert_eq!(
                classify_tool(
                    "shell",
                    &serde_json::json!({ "command": command }).to_string()
                ),
                VerificationObservation::Ignore,
                "unguarded pipeline was accepted: {command}"
            );
        }
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"command":"bash -o pipefail -c 'cargo test 2>&1 | tail -50'"}"#,
            ),
            VerificationObservation::Check(VerificationFamily::Test)
        );
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"command":"cargo fmt --check; echo fmt=$?; cargo test | tail; echo test=${PIPESTATUS[0]}"}"#,
            ),
            VerificationObservation::Ignore,
            "printing masked statuses does not preserve the shell exit status"
        );
        assert_eq!(
            classify_tool(
                "shell",
                r#"{"command":"cargo fmt --check && cargo clippy -- -D warnings && cargo test"}"#,
            ),
            VerificationObservation::Check(VerificationFamily::Lint),
            "a fail-fast AND chain preserves failure"
        );
    }

    #[test]
    fn fail_fast_multiline_checks_are_trusted_without_accepting_masked_failures() {
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({
                    "command": "set -eu\npython -m unittest discover -v\npython verify_signatures.py"
                })
                .to_string()
            ),
            VerificationObservation::Check(VerificationFamily::Test),
            "an errexit-guarded sequence preserves the first failing exit status"
        );
        for command in [
            "set -e\ncargo test || true",
            "set -e\ncargo test | tail -20",
            "cargo test\ncargo fmt --check",
            "set -e\nset +e\ncargo test\ntrue",
            "set -e\n! cargo test",
            "set -e\nif cargo test\nthen\n  true\nfi",
            "set -e\ncargo test &\ntrue",
            "set -e\n# mentioning pipefail is not enabling it\ncargo test | tail -20",
            "set -eo pipefail\ncargo test | tail -20\nset +o pipefail\ncargo test | tail -20",
        ] {
            assert_eq!(
                classify_tool(
                    "shell",
                    &serde_json::json!({ "command": command }).to_string()
                ),
                VerificationObservation::Ignore,
                "masked or non-fail-fast sequence was accepted: {command}"
            );
        }
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({
                    "command": "set -euo pipefail\ncargo test 2>&1 | tee test.log"
                })
                .to_string()
            ),
            VerificationObservation::Check(VerificationFamily::Test),
            "a pipeline with active pipefail preserves the test exit status"
        );
    }

    #[test]
    fn heredoc_program_syntax_does_not_hide_trustworthy_shell_verification() {
        let benchmark_shape = r#"set -eu
python - <<'PY'
for filename in files:
    if filename.endswith(".py"):
        print("tests passed; signatures unchanged")
PY
python -m compileall -q reservations tests
python -m unittest discover -v
git diff --check
"#;
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({ "command": benchmark_shape }).to_string()
            ),
            VerificationObservation::Check(VerificationFamily::Test),
            "control-flow tokens inside a heredoc are not shell status masking"
        );

        let one_fail_fast_chain = r#"python -m compileall -q reservations tests && python -m unittest discover -v && python - <<'PY'
if tests_are_pristine:
    print("public surface matches base; tests unchanged")
PY
"#;
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({ "command": one_fail_fast_chain }).to_string()
            ),
            VerificationObservation::Check(VerificationFamily::Test),
            "one top-level AND chain remains fail-fast even when its final command uses a heredoc"
        );

        let masked_after_heredoc = r#"set -e
python - <<'PY'
print("probe")
PY
python -m unittest discover -v || true
"#;
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({ "command": masked_after_heredoc }).to_string()
            ),
            VerificationObservation::Ignore
        );
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({
                    "command": "bash -c 'python -m unittest discover -v; true'"
                })
                .to_string()
            ),
            VerificationObservation::Ignore,
            "quoted source passed to a shell -c remains executable control flow"
        );
        for command in [
            "bash -ec 'python -m unittest discover -v; true'",
            "env VERIFY=1 /bin/sh -lc 'python -m unittest discover -v; true'",
        ] {
            assert_eq!(
                classify_tool(
                    "shell",
                    &serde_json::json!({ "command": command }).to_string()
                ),
                VerificationObservation::Ignore,
                "combined shell flags and env wrappers must not hide executable control flow"
            );
        }
        assert_eq!(
            classify_tool(
                "shell",
                &serde_json::json!({
                    "command": "echo \"$(python -m unittest discover -v; true)\""
                })
                .to_string()
            ),
            VerificationObservation::Ignore,
            "command substitution inside double quotes can still mask a failed verification"
        );
    }

    #[test]
    fn observational_work_never_requires_a_mutating_redrive() {
        assert_eq!(
            CompletionContract::production().decide(
                TaskIntent::ReadOnlyReview,
                0,
                CompletionEvidence::default(),
            ),
            CompletionDecision::AcceptNoArtifacts
        );
    }

    #[test]
    fn failed_typecheck_is_not_cleared_by_a_successful_file_read() {
        let mut ledger = VerificationLedger::default();
        ledger.observe(
            classify_tool("shell", r#"{"command":"npx tsc --noEmit"}"#),
            false,
        );
        let checkpoint = ledger.checkpoint();
        ledger.observe(
            classify_tool("read_file", r#"{"path":"package.json"}"#),
            true,
        );

        assert!(!ledger.verified_since(checkpoint));
        assert_eq!(ledger.unresolved_summary().as_deref(), Some("typecheck"));
    }

    #[test]
    fn failed_lint_test_and_build_each_require_a_matching_success() {
        for (failed, unrelated, matching, label) in [
            ("npm run lint", "npm test", "npm run lint", "lint"),
            ("cargo test", "git diff", "cargo test", "test"),
            ("cargo build", "cat Cargo.toml", "cargo build", "build"),
        ] {
            let mut ledger = VerificationLedger::default();
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{failed}"}}"#)),
                false,
            );
            let checkpoint = ledger.checkpoint();
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{unrelated}"}}"#)),
                true,
            );
            assert!(
                !ledger.verified_since(checkpoint),
                "{label} cleared by {unrelated}"
            );
            ledger.observe(
                classify_tool("shell", &format!(r#"{{"command":"{matching}"}}"#)),
                true,
            );
            assert!(
                ledger.verified_since(checkpoint),
                "successful {label} did not clear failure"
            );
        }
    }
}
