//! The presenter seam (ADR-0004): `forge-core` emits [`PresenterEvent`]s and asks for
//! permission confirmations through the [`Presenter`] trait, never touching a concrete
//! UI. v0.1 ships the [`HeadlessPresenter`] (line output for scripting/pipes/CI); the
//! ratatui+crossterm interactive renderer is the next increment behind this same trait.

pub use forge_types::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, ReplayItem, NO_ANSWER};

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
pub use answer::resolve_answer;
pub mod app;
mod app_remote;
mod arg_parse;
mod commands;
pub mod config_editor;
mod driver;
mod headless;
mod heartbeat_args;
mod help;
pub mod init_wizard;
mod keybind_configurator;
pub mod keybinds;
mod overlays;
mod refine_args;
mod render;
pub mod select;
mod stream_json;
pub mod throughput;
pub use stream_json::StreamJsonPresenter;
mod surface;
mod transcript;
mod tui;
mod voice;
mod workflow_view;
pub use app::{
    banner_lines, handle_key, input_cursor_up, insert_voice_transcript, lattice_view_lines,
    print_banner_direct, render_mesh_overlay, render_usage_overlay, render_voice_overlay,
    ActivityKind, ActivityStatus, App, InputOutcome, KeyKind, MeshCandRow, MeshOverlay,
    MeshQuotaRow, TranscriptRow, TranscriptView, UsageOverlay, UsagePaceNote, VoiceOverlay,
    VoicePhase,
};
pub use app_remote::{
    picker_kind_wire, DiffFileSnapshot, DiffHunkSnapshot, DiffSnapshot, OverlayRowSnapshot,
    OverlaySnapshot, RemoteSnapshot, TranscriptKind,
};
pub use commands::{
    arg_values, at_token_at, filter_commands, parse_command, slash_token_at, AtPathPicker, AtToken,
    Command, CommandAction, HeartbeatAction, Palette, PaletteEntry, Picker, PickerKind, PickerRow,
    RefineAction, RemoteMode, SlashToken, StatuslineAction, WorkflowAction, COMMANDS,
};
pub use config_editor::{ConfigAction, ConfigEditor, RowKind, SettingRow};
pub use driver::{ChannelPresenter, InputEvent, MouseKind, Tui, UiMsg};
pub use headless::HeadlessPresenter;
pub use help::{run_help, HelpTab};
pub use init_wizard::{BridgeItem, ProviderItem, WizardInput, WizardOutcome};
pub use keybind_configurator::run_keybind_configurator;
/// A styled scrollback line, re-exported so binaries can route out-of-band output to the right
/// sink (native scrollback inline, or the transcript log full-screen) without depending on ratatui.
pub use ratatui::text::Line as ScrollbackLine;
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
        });
        p.emit(PresenterEvent::Done {
            final_text: "done".into(),
            stop_reason: forge_types::StopReason::FinalAnswer,
        });

        let raw = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        let lines: Vec<&str> = raw.lines().filter(|l| !l.trim().is_empty()).collect();
        assert_eq!(lines.len(), 6, "one NDJSON object per emit; got:\n{raw}");

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
        // terminal result event.
        assert_eq!(parsed[5]["type"], "result");
        assert_eq!(parsed[5]["result"], "done");
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
