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
            main_login: false,
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

/// A run in progress: a resumed run has already passed the welcome, a full run starts on it.
fn screen_with(machine: Machine, mode: PlanMode) -> Screen {
    let mut state = OnboardingState::new();
    if mode == PlanMode::Resume {
        state.mark_done(StepId::Welcome);
    }
    screen_for(machine, mode, state)
}

fn screen_for(machine: Machine, mode: PlanMode, state: OnboardingState) -> Screen {
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

fn finish(s: &mut Screen, job: u64, events: Vec<JobEvent>, state: JobState) -> Vec<Effect> {
    s.apply_poll(&poll(job, state, events))
}

fn done(job: u64, result: Option<ActionResult>) -> JobEvent {
    JobEvent::Done {
        job,
        summary: "ok".into(),
        result,
    }
}

const SIGN_IN_URL: &str = "https://claude.ai/oauth/authorize?code=true&client_id=abc&state=xyz";

#[test]
fn connecting_claude_is_one_action_with_the_url_shown_and_a_code_typed_back_masked() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    let effects = press(&mut s, KeyCode::Char('t'));
    assert_eq!(
        effects,
        [Effect::Run {
            action: Action::ConnectClaude,
            purpose: Purpose::ConnectClaude
        }]
    );
    s.job_started(5, Purpose::ConnectClaude, "Connecting Claude Code".into());
    finish(
        &mut s,
        5,
        vec![JobEvent::Prompt {
            job: 5,
            prompt: forge_onboarding::PromptKind::OpenUrl {
                url: SIGN_IN_URL.into(),
            },
            message: "Open this page to sign in.".into(),
        }],
        JobState::Running,
    );
    let shown = text(&s, 100, 40);
    assert!(shown.contains("Sign in"), "{shown}");
    assert!(
        shown.contains("https://claude.ai/oauth/authorize"),
        "{shown}"
    );
    assert!(shown.contains('█'), "a QR code fits at 100x40:\n{shown}");
    assert_eq!(
        press(&mut s, KeyCode::Char('o')),
        [Effect::OpenUrl(SIGN_IN_URL.into())]
    );

    finish(
        &mut s,
        5,
        vec![JobEvent::Prompt {
            job: 5,
            prompt: forge_onboarding::PromptKind::Code {
                label: "Paste the code from your browser".into(),
            },
            message: "Paste code here if prompted >".into(),
        }],
        JobState::Running,
    );
    assert!(
        s.entry.is_none(),
        "the page's keys keep working until the code is wanted"
    );
    press(&mut s, KeyCode::Enter);
    assert_eq!(s.entry.as_ref().unwrap().target, SecretTarget::JobCode(5));
    type_text(&mut s, "CODE-FROM-BROWSER");
    assert!(
        !text(&s, 100, 40).contains("CODE-FROM-BROWSER"),
        "the code is masked"
    );
    let effects = press(&mut s, KeyCode::Enter);
    assert_eq!(
        effects,
        [Effect::Input {
            job: 5,
            text: "CODE-FROM-BROWSER".into()
        }]
    );

    let effects = finish(
        &mut s,
        5,
        vec![done(
            5,
            Some(ActionResult::Verified {
                source: "claude-cli".into(),
                status: VerifyStatus::Verified,
                detail: None,
            }),
        )],
        JobState::Succeeded,
    );
    assert!(effects.contains(&Effect::Redetect));
    assert!(
        s.sign_in.is_none(),
        "the sign-in page goes away when the job ends"
    );
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Ok);
}

#[test]
fn the_sign_in_page_fits_80x24_and_offers_the_qr_on_demand() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    s.job_started(5, Purpose::ConnectClaude, "Connecting Claude Code".into());
    finish(
        &mut s,
        5,
        vec![JobEvent::Prompt {
            job: 5,
            prompt: forge_onboarding::PromptKind::OpenUrl {
                url: SIGN_IN_URL.into(),
            },
            message: "x".into(),
        }],
        JobState::Running,
    );
    let small = draw(&s, 80, 24);
    let joined = small.join("\n");
    assert!(joined.contains("Press v for a QR code"), "{joined}");
    assert!(small[22].contains("Open the page"), "{}", small[22]);
    press(&mut s, KeyCode::Char('v'));
    assert!(s.sign_in.as_ref().unwrap().qr);
    assert!(text(&s, 80, 24).contains('█'));
}

const CLAUDE_URL: &str = "https://claude.com/cai/oauth/authorize?code=true&client_id=9d1c250a-e61b-44d9-88ed-5944d1962f5e&response_type=code&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&scope=user%3Ainference&code_challenge=ggyp0-bc8nZj3BxEEhvyhGq-sNI00PD9NNGDQ7c5yCU&code_challenge_method=S256&state=rfqXDL-weMT7TvTE0bhcsqV81PbaD5E5s0ijwClgKE8";

fn claude_sign_in_over_ssh() -> Screen {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    s.can_open_browser = false;
    s.rich_glyphs = true;
    go_to_step(&mut s, StepId::Models);
    s.job_started(5, Purpose::ConnectClaude, "Connecting Claude Code".into());
    let code = || JobEvent::Prompt {
        job: 5,
        prompt: forge_onboarding::PromptKind::Code {
            label: "Paste the code from your browser".into(),
        },
        message: "Paste code here if prompted >".into(),
    };
    finish(
        &mut s,
        5,
        vec![
            JobEvent::Prompt {
                job: 5,
                prompt: forge_onboarding::PromptKind::OpenUrl {
                    url: CLAUDE_URL.into(),
                },
                message: "Open this page to sign in.".into(),
            },
            code(),
            code(),
        ],
        JobState::Running,
    );
    s
}

#[test]
fn over_ssh_the_link_is_copied_not_opened_and_a_paste_lands_in_the_code_field() {
    let mut s = claude_sign_in_over_ssh();
    let shown = text(&s, 200, 40);
    assert!(shown.contains("press c to copy"), "{shown}");
    assert!(
        !shown.contains("Open the page"),
        "no browser over SSH:\n{shown}"
    );
    assert!(shown.contains("Paste code"), "{shown}");
    assert!(press(&mut s, KeyCode::Char('o')).is_empty());
    assert_eq!(
        press(&mut s, KeyCode::Char('c')),
        [Effect::Copy(CLAUDE_URL.into())]
    );
    assert_eq!(
        s.log
            .iter()
            .filter(|l| l.contains("Paste code here"))
            .count(),
        1,
        "a redrawn prompt line is logged once"
    );
    s.paste("abc#DEF-123\n");
    assert_eq!(s.entry.as_ref().unwrap().buf, "abc#DEF-123");
    finish(&mut s, 5, vec![], JobState::Running);
    assert_eq!(
        press(&mut s, KeyCode::Enter),
        [Effect::Input {
            job: 5,
            text: "abc#DEF-123".into()
        }]
    );
}

#[test]
fn the_code_field_is_never_pushed_off_screen_by_the_qr_code() {
    let mut s = claude_sign_in_over_ssh();
    press(&mut s, KeyCode::Enter);
    press(&mut s, KeyCode::Esc);
    assert!(s.entry.is_none() && s.sign_in.as_ref().unwrap().code.is_some());
    press(&mut s, KeyCode::Char('v'));
    press(&mut s, KeyCode::Char('p'));
    let rows = draw(&s, 200, 40);
    let joined = rows.join("\n");
    assert!(
        joined.contains("Paste the code from your browser"),
        "{joined}"
    );
}

#[test]
fn the_qr_view_shrinks_a_real_claude_link_to_fit_the_window() {
    let is_qr = |c: char| matches!(c, '█' | '▀' | '▄') || ('\u{1FB00}'..='\u{1FB3B}').contains(&c);
    let mut s = claude_sign_in_over_ssh();
    press(&mut s, KeyCode::Char('v'));
    for (w, h, sextants) in [(200, 40, false), (120, 30, true)] {
        let rows = draw(&s, w, h);
        let joined = rows.join("\n");
        assert!(
            !joined.contains("needs a bigger window"),
            "{w}x{h}:\n{joined}"
        );
        let qr: Vec<usize> = (0..rows.len())
            .filter(|&i| rows[i].chars().skip(30).any(is_qr))
            .collect();
        assert!(qr.len() > 15, "{w}x{h}:\n{joined}");
        assert!(
            *qr.last().unwrap() < h as usize - 2,
            "the QR ends above the footer at {w}x{h}:\n{joined}"
        );
        let uses_sextants = joined
            .chars()
            .any(|c| ('\u{1FB00}'..='\u{1FB3B}').contains(&c));
        assert_eq!(uses_sextants, sextants, "{w}x{h}:\n{joined}");
    }
}

#[test]
fn escape_cancels_a_sign_in_in_progress() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    s.job_started(9, Purpose::ConnectClaude, "Connecting Claude Code".into());
    finish(
        &mut s,
        9,
        vec![JobEvent::Prompt {
            job: 9,
            prompt: forge_onboarding::PromptKind::OpenUrl {
                url: SIGN_IN_URL.into(),
            },
            message: "x".into(),
        }],
        JobState::Running,
    );
    assert_eq!(press(&mut s, KeyCode::Esc), [Effect::Cancel(9)]);
}

#[test]
fn when_the_token_cannot_be_captured_the_paste_field_opens() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Resume);
    go_to_step(&mut s, StepId::Models);
    s.job_started(2, Purpose::ConnectClaude, "Connecting Claude Code".into());
    finish(
        &mut s,
        2,
        vec![done(
            2,
            Some(ActionResult::PasteNeeded {
                slot: forge_onboarding::SecretSlot::ClaudeOauthToken,
            }),
        )],
        JobState::Succeeded,
    );
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

fn reconnect_machine() -> Machine {
    let mut m = machine(Some(Login::SignedOut), true);
    m.bridges[0].main_login = true;
    m
}

#[test]
fn an_existing_user_with_claude_but_no_token_is_sent_to_a_one_click_reconnect_first() {
    let mut s = screen_with(reconnect_machine(), PlanMode::Resume);
    assert_eq!(
        s.step(),
        StepId::Models,
        "the reconnect leads even though another model works"
    );
    let card = s
        .cards()
        .into_iter()
        .find(|c| c.id == "claude-cli")
        .unwrap();
    assert!(
        card.detail.contains("Reconnect Claude Code"),
        "{}",
        card.detail
    );
    assert!(card.detail.contains("keeps your main Claude login safe"));
    s.cursor = s.cards().iter().position(|c| c.id == "claude-cli").unwrap();
    assert_eq!(s.enter_label(), "Sign in");
    let effects = press(&mut s, KeyCode::Enter);
    assert_eq!(
        effects,
        [Effect::Run {
            action: Action::ConnectClaude,
            purpose: Purpose::ConnectClaude
        }],
        "Enter runs the whole flow, no hint to read first"
    );
}

#[test]
fn recommended_setup_runs_each_item_in_turn_and_lands_on_the_summary() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    assert_eq!(s.enter_label(), "Set up everything recommended");
    let mut effects = press(&mut s, KeyCode::Enter);
    let mut ran = Vec::new();
    for _ in 0..12 {
        let Some(Effect::Run {
            action,
            purpose: Purpose::Batch(i),
        }) = effects
            .iter()
            .find(|e| {
                matches!(
                    e,
                    Effect::Run {
                        purpose: Purpose::Batch(_),
                        ..
                    }
                )
            })
            .cloned()
        else {
            break;
        };
        ran.push(action);
        assert!(text(&s, 100, 30).contains("Setting everything up"));
        finish(&mut s, 100 + i as u64, vec![], JobState::Running);
        s.job_started(100 + i as u64, Purpose::Batch(i), "x".into());
        effects = finish(
            &mut s,
            100 + i as u64,
            vec![done(100 + i as u64, Some(ActionResult::Changed))],
            JobState::Succeeded,
        );
    }
    assert!(ran.iter().any(|a| matches!(a, Action::SetBehavior { .. })));
    assert!(ran
        .iter()
        .any(|a| matches!(a, Action::ImportConfig { tool } if tool == "claude")));
    assert!(ran
        .iter()
        .any(|a| matches!(a, Action::SetReporting { mode } if mode == "auto")));
    assert!(
        !ran.iter().any(|a| matches!(a, Action::ImportMcp)),
        "choices are left to their steps"
    );
    assert_eq!(s.step(), StepId::Done);
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Ok);
    assert!(s.batch.is_none());
}

#[test]
fn recommended_setup_with_no_model_does_the_rest_then_stops_at_the_model_choice() {
    let mut s = screen_with(machine(None, false), PlanMode::Full);
    let mut effects = press(&mut s, KeyCode::Enter);
    for _ in 0..12 {
        let Some(Effect::Run {
            purpose: Purpose::Batch(i),
            ..
        }) = effects
            .iter()
            .find(|e| {
                matches!(
                    e,
                    Effect::Run {
                        purpose: Purpose::Batch(_),
                        ..
                    }
                )
            })
            .cloned()
        else {
            break;
        };
        s.job_started(200 + i as u64, Purpose::Batch(i), "x".into());
        effects = finish(
            &mut s,
            200 + i as u64,
            vec![done(200 + i as u64, Some(ActionResult::Changed))],
            JobState::Succeeded,
        );
    }
    assert_eq!(s.step(), StepId::Models, "a real choice stops the run");
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Info);
}

#[test]
fn a_failed_recommended_item_does_not_stop_the_others_and_is_reported() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    let mut effects = press(&mut s, KeyCode::Enter);
    let mut first = true;
    for _ in 0..12 {
        let Some(Effect::Run {
            purpose: Purpose::Batch(i),
            ..
        }) = effects
            .iter()
            .find(|e| {
                matches!(
                    e,
                    Effect::Run {
                        purpose: Purpose::Batch(_),
                        ..
                    }
                )
            })
            .cloned()
        else {
            break;
        };
        s.job_started(300 + i as u64, Purpose::Batch(i), "x".into());
        effects = if first {
            first = false;
            finish(
                &mut s,
                300 + i as u64,
                vec![JobEvent::Failed {
                    job: 300 + i as u64,
                    error: "disk full".into(),
                }],
                JobState::Failed,
            )
        } else {
            finish(
                &mut s,
                300 + i as u64,
                vec![done(300 + i as u64, Some(ActionResult::Changed))],
                JobState::Succeeded,
            )
        };
    }
    assert_eq!(s.notice.as_ref().unwrap().kind, NoticeKind::Warn);
    assert!(s.notice.as_ref().unwrap().text.contains("disk full"));
}

#[test]
fn stepping_through_by_hand_is_still_one_key_away_from_the_welcome() {
    let mut s = screen_with(machine(Some(Login::SignedIn), true), PlanMode::Full);
    press(&mut s, KeyCode::Char('n'));
    assert_eq!(s.step(), StepId::Models);
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

#[test]
fn the_session_start_card_connects_with_enter_defers_with_l_and_remembers_n() {
    let mut s = screen_with(reconnect_machine(), PlanMode::Resume);
    s.reconnect_card = true;
    let shown = text(&s, 80, 24);
    assert!(shown.contains("Connect Claude Code"), "{shown}");
    assert!(shown.contains("token of its own"), "{shown}");
    assert!(
        !shown.contains("step 2 of"),
        "the card is not a numbered flow:\n{shown}"
    );
    let chips = draw(&s, 80, 24)[22].clone();
    assert!(
        chips.contains("Connect now")
            && chips.contains("Later")
            && chips.contains("Don't ask again"),
        "{chips}"
    );

    let effects = press(&mut s, KeyCode::Enter);
    assert_eq!(
        effects,
        [Effect::Run {
            action: Action::ConnectClaude,
            purpose: Purpose::ConnectClaude
        }]
    );
    s.job_started(1, Purpose::ConnectClaude, "Connecting Claude Code".into());
    assert!(
        press(&mut s, KeyCode::Enter).is_empty(),
        "no second flow while one runs"
    );
    finish(
        &mut s,
        1,
        vec![done(1, Some(ActionResult::Changed))],
        JobState::Succeeded,
    );
    assert_eq!(
        s.exit,
        Some(Exit::Finished),
        "a connected card closes itself"
    );

    let mut later = screen_with(reconnect_machine(), PlanMode::Resume);
    later.reconnect_card = true;
    press(&mut later, KeyCode::Char('l'));
    assert_eq!(later.exit, Some(Exit::Left));
    assert!(!later.card_dismissed);

    let mut never = screen_with(reconnect_machine(), PlanMode::Resume);
    never.reconnect_card = true;
    press(&mut never, KeyCode::Char('n'));
    assert_eq!(never.exit, Some(Exit::Left));
    assert!(never.card_dismissed);
}

#[test]
fn a_first_run_opens_on_the_welcome_with_the_one_key_setup_for_everyone() {
    let fresh = screen_for(
        machine(None, false),
        PlanMode::Resume,
        OnboardingState::new(),
    );
    assert_eq!(fresh.step(), StepId::Welcome);
    assert_eq!(fresh.enter_label(), "Set up everything recommended");
    let shown = text(&fresh, 100, 30);
    assert!(shown.contains("No model is connected yet"), "{shown}");

    let existing = screen_for(
        machine(Some(Login::SignedIn), true),
        PlanMode::Resume,
        OnboardingState::new(),
    );
    assert_eq!(existing.step(), StepId::Welcome);
    let shown = text(&existing, 100, 30);
    assert!(
        shown.contains("Press f to finish now"),
        "a returning user can finish in two keys:\n{shown}"
    );
}

#[test]
fn a_failure_shows_the_line_that_names_the_problem_not_the_closing_hint() {
    use super::plumbing::best_error_line;
    let output =
        "installed unit\nError: systemctl not found\nNext: run `forge doctor` for a health check";
    assert_eq!(best_error_line(output), "Error: systemctl not found");
    assert_eq!(best_error_line("one\ntwo"), "two");
    assert_eq!(best_error_line(""), "It did not finish.");
}

#[test]
fn the_cursor_follows_a_card_that_moves_up_the_list_when_it_becomes_ready() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    let index = s.cards().iter().position(|c| c.id == "cerebras").unwrap();
    s.cursor = index;
    press(&mut s, KeyCode::Char('k'));
    press(&mut s, KeyCode::Char('j'));
    assert_eq!(s.cards()[s.cursor].id, "cerebras");

    let mut after = machine(None, false);
    after
        .keys
        .iter_mut()
        .find(|k| k.id == "cerebras")
        .unwrap()
        .configured = true;
    s.machine_updated(after);
    assert_eq!(
        s.cards()[s.cursor].id,
        "cerebras",
        "Enter now continues instead of opening another card"
    );
    assert_eq!(s.cursor, 0, "the ready card leads the list");
}

#[test]
fn a_long_notice_is_cut_to_the_width_not_wrapped_into_the_chips() {
    let mut s = screen_with(machine(None, false), PlanMode::Resume);
    s.notify(NoticeKind::Err, "x".repeat(300));
    let rows = draw(&s, 80, 24);
    assert!(
        rows[22].contains("Enter"),
        "chips stay on the last row: {}",
        rows[22]
    );
    assert!(rows[21].contains('…'), "{}", rows[21]);
}
