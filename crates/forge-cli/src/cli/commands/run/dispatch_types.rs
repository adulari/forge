use super::*;

pub(crate) fn project_setup_turn() -> (String, Vec<String>, Option<forge_types::TaskTier>) {
    (
        "Set up Forge for this repository completely and safely. First inspect the real repository (README, manifests, CI, source layout, and existing conventions) with non-mutating inspection tools; do not guess. Then create ONLY missing Forge setup files, never overwrite any existing file: (1) a concise, tailored root `AGENTS.md` covering overview, verified build/test/lint commands, architecture, and conventions; (2) one or more useful tailored custom agent definitions under `.forge/agents/`; and (3) a minimal `.forge/config.toml` only when it adds a verified project-specific setting. Create directories as needed. Do not modify product source, tests, dependencies, or existing guidance. Finish by reporting exactly what you created."
            .to_string(),
        Vec::new(),
        Some(forge_types::TaskTier::Complex),
    )
}

#[cfg(test)]
mod project_setup_tests {
    use super::project_setup_turn;
    use forge_core::{turn_contract::TurnContract, TaskIntent};
    use forge_types::PermissionMode;

    #[test]
    fn project_setup_turn_is_mutating() {
        let (prompt, _, _) = project_setup_turn();
        let contract = TurnContract::derive(&prompt, PermissionMode::AcceptEdits, false);

        assert_eq!(contract.intent(), TaskIntent::Mutating);
    }
}

/// The agentic project setup turn used by `/init`, the session banner, and the opt-in automatic
/// setup. It only creates missing Forge files after inspecting the actual repository.
pub(crate) fn project_setup_outcome() -> DispatchOutcome {
    let (prompt, guidance, tier) = project_setup_turn();
    DispatchOutcome::RunTurn {
        prompt,
        guidance,
        tier,
    }
}

/// What a line typed at the chat prompt means.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ChatAction {
    Quit,
    Skip,
    Run(String),
}

pub(crate) fn chat_action(line: &str) -> ChatAction {
    let t = line.trim();
    match t {
        "" => ChatAction::Skip,
        "/quit" | "/exit" | "/q" => ChatAction::Quit,
        // `//foo` escapes to a literal `/foo` prompt — mirrors the TUI behaviour.
        _ if t.starts_with("//") => ChatAction::Run(format!("/{}", &t[2..])),
        // Slash commands are TUI-only in plain mode; print a hint and skip.
        _ if t.starts_with('/') => {
            let cmd = t.split_whitespace().next().unwrap_or(t);
            eprintln!("⚒ '{cmd}' is a TUI-only command — run `forge chat` (without --plain) for the interactive TUI.");
            ChatAction::Skip
        }
        task => ChatAction::Run(task.to_string()),
    }
}

/// What the render loop must do after [`dispatch_command`].
pub(crate) enum DispatchOutcome {
    /// Command fully handled in-loop (palette, picker, note, …) — keep going.
    Handled,
    /// `/quit` — exit the TUI.
    Quit,
    /// A file command/skill expanded into a model turn the caller should spawn.
    RunTurn {
        prompt: String,
        guidance: Vec<String>,
        tier: Option<forge_types::TaskTier>,
    },
    /// `/compact` — summarize older messages in a background task (it makes a model call).
    RunCompact,
    /// `/refine [--global] [instructions]` — propose + apply one harness refinement batch in a
    /// background task (it makes a model call), exactly like `/compact`.
    RunRefine {
        instructions: Option<String>,
        global: bool,
    },
    /// `/btw <question>` — answer a side question in a background task (it makes a model call,
    /// but the answer never joins the transcript — side-questions.md).
    RunBtw { question: String },
    /// `/workflow run <name> [args]` — run a saved workflow script directly in a background task
    /// (docs/rfcs/forge-workflow.md), skipping the authoring turn entirely.
    RunSavedWorkflow {
        name: String,
        args: serde_json::Value,
    },
    /// `/loop <task>` — run the task, then re-run each turn until the model signals completion.
    /// `gates`/`max_tokens`/`max_minutes` are the autonomous quality-gate/budget options
    /// (docs/features/autonomous-gates.md) — empty/`None` for the bare form.
    StartLoop {
        prompt: String,
        gates: Vec<String>,
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    },
    /// `/goal <objective>` — decompose into a tracked task plan, then keep re-running turns
    /// autonomously until every task is done and the model signals completion. Same gate/budget
    /// fields as [`DispatchOutcome::StartLoop`].
    StartGoal {
        prompt: String,
        goal: String,
        gates: Vec<String>,
        max_tokens: Option<u64>,
        max_minutes: Option<u64>,
    },
    /// `/duel <task>` — race 2-3 mesh models on the same task, each in its own worktree
    /// (docs/features/duel.md), in a background task like a turn.
    RunDuel {
        spec: forge_core::duel_judge::DuelSpec,
    },
    /// `/mesh` — overlay opened immediately; receiver delivers the computed `MeshOverlay` (None =
    /// no catalog).
    PendingMesh(tokio::sync::oneshot::Receiver<Option<forge_tui::MeshOverlay>>),
    /// `/usage` — overlay opened immediately; receiver delivers `BridgeStats` when ready.
    PendingUsage(tokio::sync::oneshot::Receiver<bridge_stats::BridgeStats>),
    /// `/remote [--lan|--local|--anywhere]` — toggle remote control on (start the server) or off
    /// (stop it). The [`remote::Exposure`] selects bind address / public-tunnel mode.
    ToggleRemote { exposure: remote::Exposure },
    /// `/copy [N]` — write the resolved assistant response text to the clipboard. The driver loop
    /// owns the `arboard::Clipboard`, so dispatch resolves the text and hands it back to copy.
    CopyToClipboard(String),
    /// `/voice` — the render loop's `App::voice` is already set (rendering-only state); this
    /// carries the real system resources (recorder thread / download channels), which must live
    /// loop-local, not on `App` (which stays `Clone + Default`). See `apply_voice_start`.
    PendingVoice(VoiceStart),
}

/// What `/voice` produced, handed from [`dispatch_command`] back to the render loop.
pub(crate) enum VoiceStart {
    /// The whisper model was already on disk — recording started immediately.
    Recording {
        handle: forge_voice::RecordingHandle,
        model_path: std::path::PathBuf,
    },
    /// First use: the model isn't on disk yet. A background task is downloading it (progress on
    /// `progress_rx`) and will start recording once it lands (result on `done_rx`).
    Downloading {
        model_path: std::path::PathBuf,
        progress_rx: tokio::sync::watch::Receiver<(u64, Option<u64>)>,
        done_rx: tokio::sync::oneshot::Receiver<
            std::result::Result<forge_voice::RecordingHandle, String>,
        >,
    },
    /// Failed synchronously (e.g. no microphone device) — `App::voice` is already the error card
    /// (the message lives there, not duplicated here).
    Error,
}
