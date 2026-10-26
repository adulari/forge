//! Onboarding screen tests: state transitions, effects, secrets, and `TestBackend` renders at
//! 80x24 and 120x40.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use forge_onboarding::machine::{BridgeFacts, KeyFacts, LocalRecommendation, OauthFacts};
use forge_onboarding::{
    plan, Action, ActionResult, JobEvent, JobPoll, JobState, Login, Machine, OnboardingState,
    PlanMode, Session, StepId, Surface, VerifyStatus,
};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn press(screen: &mut Screen, code: KeyCode) -> Vec<Effect> {
    screen.key(key(code))
}

fn type_text(screen: &mut Screen, text: &str) {
    for c in text.chars() {
        press(screen, KeyCode::Char(c));
    }
}

#[allow(clippy::field_reassign_with_default)]
fn machine(claude: Option<Login>, groq_key: bool) -> Machine {
    let mut m = Machine::default();
    m.os = "linux".into();
    m.forge.installed = true;
    m.forge.version = "2.23.6".into();
    m.bridges = vec![
        BridgeFacts {
            id: "claude-cli".into(),
            name: "Claude Code".into(),
            binary: "claude".into(),
            installed: claude.is_some(),
            version: claude.map(|_| "2.1.4".to_string()),
            login: claude.unwrap_or(Login::Unknown),
            token_stored: false,
        },
        BridgeFacts {
            id: "codex-cli".into(),
            name: "Codex".into(),
            binary: "codex".into(),
            ..BridgeFacts::default()
        },
    ];
    m.oauth = vec![OauthFacts {
        id: "codex-oauth".into(),
        name: "ChatGPT subscription".into(),
        signed_in: false,
    }];
    m.keys = [
        ("groq", "Groq", true),
        ("cerebras", "Cerebras", true),
        ("gemini", "Google Gemini", true),
        ("anthropic", "Anthropic", false),
        ("openai", "OpenAI", false),
    ]
    .iter()
    .map(|(id, name, free)| KeyFacts {
        id: id.to_string(),
        name: name.to_string(),
        configured: *id == "groq" && groq_key,
        from_env: false,
        free_tier: *free,
        key_url: Some(format!("https://example.test/{id}")),
        featured: true,
    })
    .collect();
    m.local.recommended = Some(LocalRecommendation {
        tag: "qwen2.5-coder:14b".into(),
        label: "Qwen2.5-Coder 14B".into(),
        memory_gb: 16.0,
    });
    m.imports.claude_dir = true;
    m.imports.commands = 12;
    m.imports.skills = 3;
    m.imports.mcp_servers = 2;
    m.desktop.supported = true;
    m.search.docker = true;
    m.settings.permission = "default".into();
    m.settings.credit_mode = "normal".into();
    m.settings.theme = "auto".into();
    m.settings.reporting = "auto".into();
    m.project.git_repo = true;
    m.project.cwd = "/home/me/project".into();
    m
}

fn screen_with(machine: Machine, mode: PlanMode) -> Screen {
    let state = OnboardingState::new();
    let steps = plan(&state, &machine, Surface::Tui, mode);
    let session = Session {
        surface: Surface::Tui,
        mode,
        state,
        machine,
        plan: steps,
    };
    Screen::new(session)
}

fn go_to_step(screen: &mut Screen, step: StepId) {
    screen.page = screen
        .session
        .plan
        .iter()
        .position(|s| s.id == step)
        .expect("step is in the plan");
    screen.cursor = 0;
    screen.enter_page_for_test();
}

impl Screen {
    fn enter_page_for_test(&mut self) {
        let _ = self.opening_effects();
    }
}

fn draw(screen: &Screen, w: u16, h: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render(f, screen)).unwrap();
    let buf = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect()
}

fn text(screen: &Screen, w: u16, h: u16) -> String {
    draw(screen, w, h).join("\n")
}

fn poll(job: u64, state: JobState, events: Vec<JobEvent>) -> JobPoll {
    JobPoll {
        job,
        action: "x".into(),
        state,
        next: events.len(),
        events,
    }
}

#[test]
fn a_fresh_machine_opens_on_the_model_step_and_enter_asks_for_a_key() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    assert_eq!(s.step(), StepId::Models);
    assert_eq!(s.cards()[0].id, "groq", "the first free tier leads");
    assert_eq!(s.enter_label(), "Paste a key");
    let effects = press(&mut s, KeyCode::Enter);
    assert!(effects.is_empty());
    let entry = s.entry.as_ref().expect("a masked field opens");
    assert_eq!(
        entry.target,
        SecretTarget::ApiKey {
            provider: "groq".into(),
            replace: false
        }
    );
}

#[test]
fn a_typed_key_is_masked_on_screen_and_sent_only_as_a_secret_action() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    press(&mut s, KeyCode::Enter);
    type_text(&mut s, "gsk-live-SECRET-0123456789");
    let rendered = text(&s, 100, 30);
    assert!(
        !rendered.contains("SECRET"),
        "the key must never be drawn:\n{rendered}"
    );
    assert!(rendered.contains("••••••"), "{rendered}");

    let effects = press(&mut s, KeyCode::Enter);
    let [Effect::Run { action, purpose }] = effects.as_slice() else {
        panic!("expected one job, got {effects:?}");
    };
    assert_eq!(purpose, &Purpose::StoreKey("groq".into()));
    let Action::StoreKey {
        provider,
        key,
        replace,
    } = action
    else {
        panic!("wrong action");
    };
    assert_eq!(provider, "groq");
    assert_eq!(key.expose(), "gsk-live-SECRET-0123456789");
    assert!(!replace);
    assert!(s.entry.is_none(), "the field is gone once submitted");
    assert!(
        !format!("{effects:?}").contains("SECRET"),
        "Debug output stays masked"
    );
}

#[test]
fn escape_cancels_the_entry_and_nothing_is_sent() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    press(&mut s, KeyCode::Enter);
    type_text(&mut s, "abc");
    assert!(press(&mut s, KeyCode::Esc).is_empty());
    assert!(s.entry.is_none());
}

#[test]
fn continuing_without_a_model_is_refused_with_a_reason() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    let effects = press(&mut s, KeyCode::Char('c'));
    assert!(effects.is_empty());
    assert_eq!(s.step(), StepId::Models);
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Warn);
}

#[test]
fn found_sources_are_checked_automatically_once_and_enter_continues() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    assert_eq!(
        s.step(),
        StepId::Behavior,
        "a returning user lands past the model step"
    );
    let effects = s.opening_effects();
    let verified: Vec<&str> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Run {
                action: Action::Verify { source },
                ..
            } => Some(source.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(verified, ["claude-cli", "groq"]);
    assert!(s.opening_effects().is_empty(), "never asked twice");

    go_to_step(&mut s, StepId::Models);
    assert_eq!(s.enter_label(), "Continue");
    let effects = press(&mut s, KeyCode::Enter);
    assert!(effects.contains(&Effect::Persist));
    assert_eq!(s.step(), StepId::Behavior);
    assert_eq!(
        s.session.state.status(StepId::Models),
        forge_onboarding::StepStatus::Done
    );
}

#[test]
fn a_finished_check_updates_the_badge_and_a_refusal_explains_itself() {
    let mut s = screen_with(machine(None, true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    s.job_started(1, Purpose::Verify("groq".into()), "Checking groq".into());
    let card = s.cards().into_iter().find(|c| c.id == "groq").unwrap();
    assert_eq!(s.card_status(&card), CardStatus::Checking);

    let result = ActionResult::Verified {
        source: "groq".into(),
        status: VerifyStatus::Verified,
        detail: Some("Groq answered with 12 models.".into()),
    };
    let effects = s.apply_poll(&poll(
        1,
        JobState::Succeeded,
        vec![JobEvent::Done {
            job: 1,
            summary: "ok".into(),
            result: Some(result),
        }],
    ));
    assert!(effects.contains(&Effect::Persist));
    assert_eq!(s.card_status(&card), CardStatus::Verified);

    s.job_started(2, Purpose::Verify("groq".into()), "Checking groq".into());
    s.apply_poll(&poll(
        2,
        JobState::Succeeded,
        vec![JobEvent::Done {
            job: 2,
            summary: "no".into(),
            result: Some(ActionResult::Verified {
                source: "groq".into(),
                status: VerifyStatus::Rejected,
                detail: Some("Groq refused the key (401). Check it and try again.".into()),
            }),
        }],
    ));
    assert_eq!(s.card_status(&card), CardStatus::Rejected);
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Err);
    assert_eq!(s.enter_label(), "Replace the key");
    press(&mut s, KeyCode::Enter);
    assert_eq!(
        s.entry.as_ref().unwrap().target,
        SecretTarget::ApiKey {
            provider: "groq".into(),
            replace: true
        }
    );
}

#[test]
fn a_failed_job_shows_its_last_line_and_streams_output_into_the_log() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    s.job_started(7, Purpose::Service, "Installing".into());
    s.apply_poll(&poll(
        7,
        JobState::Running,
        vec![JobEvent::Output {
            job: 7,
            line: "writing unit".into(),
        }],
    ));
    assert_eq!(s.log.back().map(String::as_str), Some("writing unit"));
    assert!(s.busy());
    s.apply_poll(&poll(
        7,
        JobState::Failed,
        vec![JobEvent::Failed {
            job: 7,
            error: "first\nsystemctl failed".into(),
        }],
    ));
    assert!(!s.busy());
    assert_eq!(s.notice.as_ref().unwrap().text, "systemctl failed");
}

#[test]
fn the_claude_token_flow_hands_over_the_terminal_then_asks_for_the_paste() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    let effects = press(&mut s, KeyCode::Char('t'));
    assert_eq!(
        effects,
        [Effect::Terminal {
            action: Action::ClaudeSetupToken,
            purpose: Purpose::ClaudeTokenFlow
        }]
    );
    let effects = s.terminal_finished(&Purpose::ClaudeTokenFlow, true);
    assert!(effects.contains(&Effect::Redetect));
    assert_eq!(s.entry.as_ref().unwrap().target, SecretTarget::ClaudeToken);
    type_text(&mut s, "sk-ant-oat01-TOKEN");
    let effects = press(&mut s, KeyCode::Enter);
    let [Effect::Run {
        action: Action::StoreSecret { slot, value },
        purpose,
    }] = effects.as_slice()
    else {
        panic!("{effects:?}");
    };
    assert_eq!(*slot, forge_onboarding::SecretSlot::ClaudeOauthToken);
    assert_eq!(value.expose(), "sk-ant-oat01-TOKEN");
    assert_eq!(purpose, &Purpose::StoreClaudeToken);
}

#[test]
fn local_models_install_ollama_when_it_is_missing_and_pull_when_it_is_not() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    let index = s.cards().iter().position(|c| c.id == "ollama").unwrap();
    s.cursor = index;
    let effects = press(&mut s, KeyCode::Enter);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Terminal {
            action: Action::OllamaInstall,
            purpose: Purpose::LocalInstall(_)
        }]
    ));
    let mut m = machine(None, false);
    m.local.ollama_installed = true;
    let mut s = screen_with(m, PlanMode::Resume);
    let index = s.cards().iter().position(|c| c.id == "ollama").unwrap();
    s.cursor = index;
    let effects = press(&mut s, KeyCode::Enter);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Run { action: Action::OllamaPull { tag }, purpose: Purpose::LocalPull(_) }]
            if tag == "qwen2.5-coder:14b"
    ));
}

#[test]
fn leaving_counts_as_a_deferral_only_when_everything_required_already_works() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(s.exit, Some(Exit::Left));
    assert_eq!(s.session.state.deferred_count, 0);

    let mut s = screen_with(machine(None, true), PlanMode::Resume);
    press(&mut s, KeyCode::Char('q'));
    assert_eq!(s.session.state.deferred_count, 1);
}

#[test]
fn a_returning_user_can_finish_from_the_welcome_in_one_key() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    assert_eq!(s.step(), StepId::Welcome);
    assert!(text(&s, 100, 30).contains("Everything Forge needs already works"));
    press(&mut s, KeyCode::Char('f'));
    assert_eq!(s.exit, Some(Exit::Finished));
    assert!(s.session.state.flow_version >= forge_onboarding::FLOW_VERSION);
}

#[test]
fn f_does_nothing_on_a_machine_that_cannot_run_yet() {
    let mut s = screen_with(machine(None, false), PlanMode::Full);
    press(&mut s, KeyCode::Char('f'));
    assert_eq!(s.exit, None);
}

#[test]
fn the_behavior_step_saves_the_chosen_temper_and_credit_mode() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Behavior);
    assert_eq!(s.ui.temper, 1, "Ask is preselected");
    press(&mut s, KeyCode::Down);
    press(&mut s, KeyCode::Right);
    press(&mut s, KeyCode::Down);
    let effects = press(&mut s, KeyCode::Enter);
    let saved = effects.iter().find_map(|e| match e {
        Effect::Run {
            action:
                Action::SetBehavior {
                    permission,
                    credit_mode,
                },
            ..
        } => Some((permission.clone(), credit_mode.clone())),
        _ => None,
    });
    assert_eq!(
        saved,
        Some(("accept-edits".to_string(), "frugal".to_string()))
    );
    assert_eq!(s.step(), StepId::WebSearch);
}

#[test]
fn imports_run_one_at_a_time_and_the_mcp_picker_gets_the_terminal() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Import);
    assert_eq!(s.ui.import_checked.len(), 3);
    let first = press(&mut s, KeyCode::Enter);
    assert!(matches!(
        first.as_slice(),
        [Effect::Run { action: Action::ImportConfig { tool }, purpose: Purpose::Import }] if tool == "claude"
    ));
    let second = s.terminal_finished(&Purpose::Import, true);
    assert!(second.iter().any(|e| matches!(
        e,
        Effect::Terminal {
            action: Action::ImportMcp,
            ..
        }
    )));
    let third = s.terminal_finished(&Purpose::Import, true);
    assert!(third.iter().any(|e| matches!(
        e,
        Effect::Run { action: Action::SetKeybindPreset { preset }, .. } if preset == "claude-code"
    )));
    let last = s.terminal_finished(&Purpose::Import, true);
    assert!(last.contains(&Effect::Redetect));
    assert_ne!(s.step(), StepId::Import, "the queue ending moves on");
}

#[test]
fn unchecking_everything_skips_straight_through() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Import);
    for i in 0..3 {
        s.cursor = i;
        press(&mut s, KeyCode::Char(' '));
    }
    assert_eq!(s.enter_label(), "Continue");
    press(&mut s, KeyCode::Enter);
    assert_ne!(s.step(), StepId::Import);
}

#[test]
fn the_reporting_step_defaults_to_automatic_and_explains_what_is_sent() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Reporting);
    assert_eq!(s.ui.reporting, 0);
    assert!(!text(&s, 100, 30).contains("Never your prompts, your code"));
    press(&mut s, KeyCode::Char('w'));
    assert!(text(&s, 100, 30).contains("Never your prompts, your code"));
    press(&mut s, KeyCode::Down);
    press(&mut s, KeyCode::Down);
    let effects = press(&mut s, KeyCode::Enter);
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::Run { action: Action::SetReporting { mode }, .. } if mode == "off"
    )));
}

#[test]
fn skipping_a_step_marks_it_skipped_and_back_returns() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Anywhere);
    press(&mut s, KeyCode::Char('s'));
    assert_eq!(
        s.session.state.status(StepId::Anywhere),
        forge_onboarding::StepStatus::Skipped
    );
    press(&mut s, KeyCode::Char('b'));
    assert_eq!(s.step(), StepId::Anywhere);
}

#[test]
fn the_last_page_finishes_only_when_a_model_works() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    go_to_step(&mut s, StepId::Done);
    press(&mut s, KeyCode::Enter);
    assert_eq!(s.exit, None);
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Warn);

    let mut s = screen_with(machine(None, true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Done);
    press(&mut s, KeyCode::Enter);
    assert_eq!(s.exit, Some(Exit::Finished));
}

// ---------------------------------------------------------------- rendering

fn every_page_screen(size: (u16, u16)) -> Vec<(StepId, Vec<String>)> {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    let steps: Vec<StepId> = s.session.plan.iter().map(|p| p.id).collect();
    steps
        .into_iter()
        .map(|step| {
            go_to_step(&mut s, step);
            (step, draw(&s, size.0, size.1))
        })
        .collect()
}

#[test]
fn every_page_fits_80x24_and_keeps_its_chips_on_the_last_row() {
    for (step, rows) in every_page_screen((80, 24)) {
        let joined = rows.join("\n");
        assert!(joined.contains("Forge setup"), "{step:?}\n{joined}");
        let chips_row = rows[22].trim_end_matches([' ', '│']).trim();
        assert!(
            chips_row.contains("Enter") && chips_row.contains("Not now"),
            "{step:?} chips row: {chips_row:?}\n{joined}"
        );
        assert!(!joined.contains("step 0"), "{joined}");
    }
}

#[test]
fn every_page_renders_at_120x40_with_the_step_rail() {
    for (step, rows) in every_page_screen((120, 40)) {
        let joined = rows.join("\n");
        assert!(
            joined.contains("Welcome") && joined.contains("Models"),
            "{step:?}\n{joined}"
        );
        assert!(joined.contains("Error reports"), "{step:?}\n{joined}");
    }
}

#[test]
fn a_narrow_terminal_collapses_the_rail_to_dots() {
    let s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    let narrow = text(&s, 80, 24);
    assert!(narrow.contains('●') && narrow.contains('○'), "{narrow}");
    assert!(
        !narrow.contains("Bring your setup"),
        "no rail under 100 columns:\n{narrow}"
    );
    let wide = text(&s, 120, 40);
    assert!(wide.contains("Bring your setup"), "{wide}");
}

#[test]
fn the_welcome_leads_with_what_was_found() {
    let s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    let t = text(&s, 100, 30);
    assert!(t.contains("Found on this machine"), "{t}");
    assert!(t.contains("Claude Code 2.1.4 · signed in"), "{t}");
    assert!(t.contains("12 commands, 3 skills, 2 MCP servers"), "{t}");
    assert!(t.contains("█▀▀"), "wordmark: {t}");
}

#[test]
fn the_model_list_shows_badges_and_scrolls_to_the_focused_card() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    s.session
        .state
        .record_verification("claude-cli", VerifyStatus::Verified, None);
    let t = text(&s, 100, 30);
    assert!(t.contains("verified"), "{t}");
    assert!(t.contains("free tier"), "{t}");
    let last = s.cards().len() - 1;
    s.cursor = last;
    let rows = draw(&s, 80, 24);
    let joined = rows.join("\n");
    assert!(joined.contains("❯"), "focused row stays visible:\n{joined}");
}

#[test]
fn a_running_job_shows_a_spinner_and_its_live_output() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Service);
    s.job_started(
        3,
        Purpose::Service,
        "Installing the background service".into(),
    );
    s.apply_poll(&poll(
        3,
        JobState::Running,
        vec![JobEvent::Output {
            job: 3,
            line: "writing the unit file".into(),
        }],
    ));
    let t = text(&s, 80, 24);
    assert!(t.contains("Installing the background service"), "{t}");
    assert!(t.contains("writing the unit file"), "{t}");
}

#[test]
fn reduced_motion_replaces_the_spinner_with_a_still_mark() {
    use crate::theme::glyphs::g;
    let theme = crate::theme::Theme {
        motion: crate::theme::Motion::Reduced,
        ..crate::theme::Theme::default()
    };
    crate::theme::scoped(theme, || {
        assert_eq!(g().spin(0), g().spin(5));
        assert_eq!(g().spin(0), g().spinner_still);
    });
}
