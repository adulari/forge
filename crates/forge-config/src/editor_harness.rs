pub(crate) fn group_and_label(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "harness.completion_contract" => Some(("Behaviour", "Completion contract")),
        "harness.stop_retries" => Some(("Behaviour", "Stop retries")),
        "harness.thinking_loop_breaker" => Some(("Behaviour", "Thinking-loop breaker")),
        "harness.repeat_interrupt" => Some(("Behaviour", "Repeat-call interrupt")),
        "harness.stream_silence_retry" => Some(("Behaviour", "Stream-silence retry")),
        "harness.wait_job" => Some(("Behaviour", "Wait for background job")),
        "harness.append_only_prefix" => Some(("Behaviour", "Append-only prompt prefix")),
        "harness.cache_monitor" => Some(("Behaviour", "Prompt-cache break warnings")),
        "harness.compaction_summary_cap" => Some(("Behaviour", "16k compaction summary")),
        "harness.edit_mode" => Some(("Behaviour", "File edit mode")),
        "harness.family_profiles" => Some(("Behaviour", "Model-family profiles")),
        "harness.eval_kernel" => Some(("Behaviour", "Persistent Python kernel")),
        "harness.lsp_nav" => Some(("Behaviour", "LSP navigation tool")),
        "harness.subagent_output_schema" => Some(("Behaviour", "Subagent output schemas")),
        "harness.advisor.enabled" => Some(("Behaviour", "Advisor review")),
        "harness.advisor.model" => Some(("Behaviour", "Advisor model")),
        "harness.advisor.timeout_secs" => Some(("Behaviour", "Advisor timeout")),
        "tui.session_strip" => Some(("Interface", "Session strip")),
        "tui.sidebar" => Some(("Interface", "Sidebar")),
        _ => None,
    }
}

pub(crate) fn help(path: &str) -> Option<&'static str> {
    Some(match path {
        "tui.session_strip" => "A one-row strip above the chat listing the daemon's other sessions: a spinner for busy, a dot for waiting on you. Alt+1-9 switches to one. /strip flips it.",
        "tui.sidebar" => "A right sidebar on terminals 120 or more columns wide: context gauge, spend, MCP health, tasks and the branch. Ctrl+B or /sidebar flips it.",
        "harness.completion_contract" => "Append a short completion contract to the end of the system prompt: do not yield while actionable work remains, smoke-run before yielding, no stubs. Default off.",
        "harness.stop_retries" => "Continue up to 3 times when a reply announces a next step but makes no tool call, and retry empty or text-only tool-call replies up to 3 times before benching. A user-pinned model is never benched for one empty reply. Wins over harness_profile = minimal. Default off.",
        "harness.thinking_loop_breaker" => "When consecutive tool-free replies keep restating the same plan, inject 'Act, don't re-plan: make the next tool call.' Wins over harness_profile = minimal. Default off.",
        "harness.edit_mode" => "How the model edits existing files. replace: edit_file and multi_edit with exact old/new text. hashline: read_file tags each line with its number under a per-file tag and edit_file takes line anchors plus that tag, which fails with the current lines if the file changed. Suits models that often miss exact matches. Default replace.",
        "harness.family_profiles" => "Apply tuned defaults per model family (Kimi, DeepSeek and so on): loop flags you have not set yourself, a short prompt delta and reasoning handling. A flag you set explicitly always wins. Default off.",
        "harness.repeat_interrupt" => "After 5 identical consecutive tool calls with identical results, tell the model not to repeat that call and to change approach; the progress ledger's cycle stop moves two repeats later. Wins over harness_profile = minimal. Default off.",
        "harness.stream_silence_retry" => "Abort a model stream that has been silent for 4 minutes (raises the idle window to 240s if it is shorter) and retry the same model up to twice before failing over. Any stream event, reasoning included, counts as activity, so long reasoning is never cut. Default off.",
        "harness.wait_job" => "Add a wait action to shell_job: block up to N seconds for a background job and return its exit code and the tail of its log, so the model waits instead of polling. Default off.",
        "harness.append_only_prefix" => "Keep the cached prompt prefix append-only: the task-list snapshot moves out of the pinned session context (which sits before the whole transcript) into a note appended when the list changes, so a changed list no longer re-bills the conversation. Default off.",
        "harness.cache_monitor" => "Warn immediately, with the cause, when a request breaks the provider prompt cache: the tool list changed or an already-sent message was rewritten. Without it the warning needs two rewrites in a row. Default off.",
        "harness.compaction_summary_cap" => "Cap a compaction summary at 16k tokens (Factory Droid's budget) instead of 20k. Default off.",
        "harness.eval_kernel" => "Advertise a `python` tool: a persistent per-session Python kernel. Runs code with shell-level permissions. Applies to new sessions. Default off.",
        "harness.lsp_nav" => "Add a deferred read-only lsp tool (definition, hover, references; Python via ty/pyright). Library definitions come back with source inline. Default off.",
        "harness.subagent_output_schema" => "Let the parent request a JSON-schema result from spawn_agents children; validated with up to 2 retries.",
        "harness.advisor.enabled" => "Before a turn that changed something finishes, a second model reads the work once and can send it back once with up to 4 notes. Skipped for CLI bridges and subscription models. Default off.",
        "harness.advisor.model" => "Model that reviews. Empty uses the model driving the turn. A Claude Sonnet-class model is the recommended setting; small models truncated on large transcripts in the replay.",
        "harness.advisor.timeout_secs" => "Seconds before an advisor call is abandoned and the turn finishes as it would have (default 120).",
        _ => return None,
    })
}
