//! The presenter seam (ADR-0004): `forge-core` emits [`PresenterEvent`]s and asks for
//! permission confirmations through the [`Presenter`] trait, never touching a concrete
//! UI. v0.1 ships the [`HeadlessPresenter`] (line output for scripting/pipes/CI); the
//! ratatui+crossterm interactive renderer is the next increment behind this same trait.

pub use forge_types::{
    Answer, ConfirmOutcome, ConfirmReply, Presenter, PresenterEvent, QChoice, Question, ReplayItem,
    NO_ANSWER,
};
pub use question_form::{parse_structured_answers, FormOutcome, QuestionForm};

/// How a model id reads to a person. A bare bridge id (`claude-cli::`) is a real, valid pin
/// meaning "whatever model that CLI is configured to use" — it is the first entry in the built-in
/// complex-tier defaults — but printed verbatim it looks like a truncated id or a bug, and it is
/// what a user with no API keys sees on their very first turn.
pub fn display_model(id: &str) -> String {
    match id.strip_suffix("::") {
        Some(provider) if !provider.is_empty() => format!("{provider} (its default model)"),
        _ => id.to_string(),
    }
}

pub mod answer;
pub mod question_form;
pub use answer::resolve_answer;
mod ansi_line;
pub mod app;
mod app_btw;
mod app_remote;
pub use app_btw::BtwOverlay;
pub mod anywhere_menu;
mod session_delete;
pub use session_delete::{DeleteKeyOutcome, SessionDeleteDialog};
mod arg_parse;
pub mod board;
mod commands;
pub mod config_editor;
mod copy_mark;
mod driver;
mod git_location;
pub mod goal_status;
mod headless;
mod heartbeat_args;
mod help;
mod json_result;
mod keybind_configurator;
pub mod keybinds;
pub mod notes_overlay;
pub mod onboarding;
mod overlays;
mod peer_message;
pub mod perf;
mod picker;
mod print_presenter;
mod prompt_history;
mod stash_args;
pub mod theme;
pub use prompt_history::PromptHistory;
mod refine_args;
mod render;
mod result_totals;
pub mod select;
mod stream_json;
mod stream_json_message;
pub mod throughput;
mod usage_source;
pub use json_result::{error_result_line, JsonResultPresenter};
pub use print_presenter::PrintPresenter;
mod shell_mode;
pub use shell_mode::bang_command;
pub use stream_json::{StreamInit, StreamInitSlot, StreamJsonPresenter};
pub use usage_source::{ModelUsage, UsageSlot, UsageSource};
mod qr_overlay;
mod surface;
mod transcript;
mod tui;
pub mod update_card;
mod voice;
mod workflow_view;
pub use app::{
    banner_lines, handle_key, input_cursor_up, insert_voice_transcript, lattice_view_lines,
    page_file, print_banner_direct, render_mesh_overlay, render_usage_overlay,
    render_voice_overlay, ActivityKind, ActivityStatus, App, ChildKey, Deferred, DeferredInbox,
    IdleInterrupt, InputOutcome, KeyKind, MeshCandRow, MeshOverlay, MeshQuotaRow, OutputAction,
    OutputView, PanelKey, TranscriptRow, TranscriptView, UsageOverlay, UsagePaceNote, VoiceOverlay,
    VoicePhase,
};
pub use app_remote::{
    picker_kind_wire, DiffFileSnapshot, DiffHunkSnapshot, DiffSnapshot, OverlayRowSnapshot,
    OverlaySnapshot, RemoteSnapshot, TranscriptKind,
};
pub use commands::{
    arg_values, at_token_at, filter_commands, parse_command, slash_token_at, AtPathPicker, AtToken,
    Command, CommandAction, GoalControl, HeartbeatAction, Palette, PaletteEntry, Picker,
    PickerKind, PickerRow, RefineAction, RemoteMode, SlashToken, StashOp, StatuslineAction,
    WorkflowAction, COMMANDS,
};
pub use config_editor::{ConfigAction, ConfigEditor, RowKind, SettingRow};
pub use driver::{
    edit_in_external_editor, external_editor_command, frame_wait, install_panic_restore,
    ChannelPresenter, InputEvent, MouseKind, Tui, UiMsg,
};
pub use headless::HeadlessPresenter;
pub use help::{run_help, HelpTab};
pub use keybind_configurator::run_keybind_configurator;
/// A styled scrollback line, re-exported so binaries can route out-of-band output to the right
/// sink (native scrollback inline, or the transcript log full-screen) without depending on ratatui.
pub use ratatui::text::Line as ScrollbackLine;
pub use render::prewarm_highlighter;
pub use select::{select_multi, select_one, SelectItem};
pub use transcript::{run_transcript_viewer, transcript_lines};
pub use tui::TuiPresenter;
pub use workflow_view::{WfPhase, WfRow, WfZoom, WorkflowView};

// The interaction contracts are owned by `forge-types`; this crate provides surface adapters.

#[cfg(test)]
mod stream_json_tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    /// A `Write` sink that captures bytes into a shared buffer the test can read afterwards.
    #[derive(Clone)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);
    impl Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stream_json_emits_parseable_ndjson_with_expected_event_types() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut p = StreamJsonPresenter::with_writer(Box::new(SharedBuf(buf.clone())));

        p.emit(PresenterEvent::SessionStarted {
            id: "sess-42".into(),
        });
        p.emit(PresenterEvent::Routing {
            effort: None,
            tier: "standard".into(),
            model: "openai::gpt-4o".into(),
            rationale: "best coding score".into(),
        });
        p.emit(PresenterEvent::AssistantDelta("Hello".into()));
        p.emit(PresenterEvent::ToolStart {
            name: "shell".into(),
            args: "{\"command\":\"ls\"}".into(),
        });
        p.emit(PresenterEvent::ToolResult {
            name: "shell".into(),
            ok: true,
            summary: "a.txt b.txt".into(),
            detail: None,
        });
        p.emit(PresenterEvent::Done {
            final_text: "done".into(),
            stop_reason: forge_types::StopReason::FinalAnswer,
        });

        let raw = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        // The final answer never streamed here, so it is sent as its own assistant message.
        assert_eq!(lines.len(), 7, "one NDJSON object per message; got:\n{raw}");

        // Every line is valid JSON.
        let parsed: Vec<serde_json::Value> = lines
            .iter()
            .map(|l| serde_json::from_str(l).expect("each line must be valid JSON"))
            .collect();

        // init event carries the session id and CC-style type/subtype.
        assert_eq!(parsed[0]["type"], "system");
        assert_eq!(parsed[0]["subtype"], "init");
        assert_eq!(parsed[0]["session_id"], "sess-42");
        // session id propagates onto later events.
        assert_eq!(parsed[2]["session_id"], "sess-42");
        // assistant text mirrors CC's message.content[].text shape.
        assert_eq!(parsed[2]["type"], "assistant");
        assert_eq!(parsed[2]["message"]["content"][0]["type"], "text");
        assert_eq!(parsed[2]["message"]["content"][0]["text"], "Hello");
        // tool_use input is embedded as parsed JSON, not a string.
        assert_eq!(parsed[3]["message"]["content"][0]["type"], "tool_use");
        assert_eq!(parsed[3]["message"]["content"][0]["input"]["command"], "ls");
        // tool_result carries the is_error flag.
        assert_eq!(parsed[4]["message"]["content"][0]["type"], "tool_result");
        assert_eq!(parsed[4]["message"]["content"][0]["is_error"], false);
        // tool_use and tool_result are paired by id.
        assert!(parsed[3]["message"]["content"][0]["id"].is_string());
        assert_eq!(
            parsed[3]["message"]["content"][0]["id"],
            parsed[4]["message"]["content"][0]["tool_use_id"]
        );
        assert_eq!(parsed[5]["type"], "assistant");
        assert_eq!(parsed[5]["message"]["content"][0]["text"], "done");
        // terminal result event.
        assert_eq!(parsed[6]["type"], "result");
        assert_eq!(parsed[6]["result"], "done");
        assert_eq!(parsed[6]["stop_reason"], "final_answer");
        assert!(parsed[6]["usage"].is_object() && parsed[6]["num_turns"].is_u64());
        assert_eq!(
            parsed[3]["message"]["content"][0]["id"],
            parsed[4]["message"]["content"][0]["tool_use_id"]
        );
    }

    #[test]
    fn machine_surfaces_decline_side_calls_they_never_render() {
        let stream = StreamJsonPresenter::with_writer(Box::new(SharedBuf(Default::default())));
        assert!(!stream.consumes_recap() && !stream.consumes_suggestions());
        let headless = HeadlessPresenter::new(false);
        assert!(headless.consumes_recap(), "headless prints the recap line");
        assert!(!headless.consumes_suggestions());
    }

    #[test]
    fn stream_json_preserves_cached_input_tokens() {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let mut p = StreamJsonPresenter::with_writer(Box::new(SharedBuf(buf.clone())));
        p.emit(PresenterEvent::SessionStarted {
            id: "cache-session".into(),
        });
        p.emit(PresenterEvent::Cost {
            session_total_usd: 0.01,
            session_in: 1_200,
            session_cached_in: Some(900),
            session_out: 40,
            context_tokens: 1_240,
            context_limit: Some(128_000),
        });

        let raw = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let usage: serde_json::Value = serde_json::from_str(raw.lines().nth(1).unwrap()).unwrap();
        assert_eq!(usage["usage"]["input_tokens"], 1_200);
        assert_eq!(usage["usage"]["cached_input_tokens"], 900);
        assert_eq!(usage["usage"]["output_tokens"], 40);
    }
}

#[cfg(test)]
mod display_model_tests {
    use super::display_model;

    #[test]
    fn a_bare_bridge_id_reads_as_the_cli_default_not_a_truncated_id() {
        assert_eq!(
            display_model("claude-cli::"),
            "claude-cli (its default model)"
        );
        assert_eq!(
            display_model("codex-cli::"),
            "codex-cli (its default model)"
        );
    }

    #[test]
    fn a_named_model_is_left_exactly_as_it_is() {
        for id in [
            "claude-cli::opus",
            "anthropic::claude-opus-5",
            "openrouter::meta/muse-spark-1.3",
            "ollama::llama3.2",
        ] {
            assert_eq!(display_model(id), id);
        }
    }

    #[test]
    fn a_degenerate_id_is_not_dressed_up() {
        // Nothing before the separator is not a provider; leave it visible rather than
        // printing " (its default model)".
        assert_eq!(display_model("::"), "::");
        assert_eq!(display_model(""), "");
    }
}
