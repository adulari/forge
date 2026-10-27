//! The model step's page: one card per source with its badge, and the help for the focused one.

use forge_onboarding::{ModelSource, SourceKind, SourceState};
use ratatui::text::{Line, Span};

use super::models::CardStatus;
use super::screen::Screen;
use super::text::{blank, heading, mask, paragraph, truncate};
use super::view::{padded, selectable};
use crate::theme::glyphs::g;
use crate::theme::styles;

// intro() lives with the other pages; the model step opens with the same heading and blurb.
fn intro(title: &str, text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![heading(title)];
    out.extend(paragraph(text, width, 0, styles::muted()));
    out.push(blank());
    out
}

pub(super) trait DetailLine {
    fn detail_line(&self) -> String;
}

impl DetailLine for ModelSource {
    fn detail_line(&self) -> String {
        if self.kind == SourceKind::ApiKey {
            format!("{} · {}", self.name, self.detail)
        } else {
            self.detail.clone()
        }
    }
}

// ---------------------------------------------------------------- models

fn badge(s: &Screen, status: CardStatus, card: &ModelSource) -> (String, ratatui::style::Style) {
    match status {
        CardStatus::Verified => ("verified".into(), styles::ok()),
        CardStatus::Checking => (
            format!("{} checking", g().spin(s.tick)),
            styles::spinner(s.tick),
        ),
        CardStatus::Unchecked => ("saved".into(), styles::muted()),
        CardStatus::Rejected => ("refused".into(), styles::err()),
        CardStatus::Ready => ("found".into(), styles::muted()),
        CardStatus::NeedsLogin => ("sign in".into(), styles::warn()),
        CardStatus::NeedsSetup if card.free_tier => ("free tier".into(), styles::ok()),
        CardStatus::NeedsSetup => ("set up".into(), styles::muted()),
        CardStatus::NotInstalled => ("not installed".into(), styles::faint()),
    }
}

fn card_glyph(s: &Screen, status: CardStatus) -> (String, ratatui::style::Style) {
    match status {
        CardStatus::Verified => (g().ok.into(), styles::ok()),
        CardStatus::Checking => (g().spin(s.tick).into(), styles::spinner(s.tick)),
        CardStatus::Unchecked | CardStatus::Ready => (g().ok.into(), styles::muted()),
        CardStatus::Rejected => (g().fail.into(), styles::err()),
        CardStatus::NeedsLogin => (g().warn.into(), styles::warn()),
        _ => (g().task_open.into(), styles::faint()),
    }
}

fn source_help(card: &ModelSource, status: CardStatus) -> String {
    let url = card.key_url.clone().unwrap_or_default();
    match (card.kind, card.state) {
        (SourceKind::ClaudeCode, SourceState::Ready) => "Uses your Claude subscription through the claude command. Forge runs it under a token of its own so it never touches your Claude login. Press t to create that token.".into(),
        (SourceKind::ClaudeCode, SourceState::NeedsLogin) => "Claude Code is installed but not signed in. Enter signs you in and creates a token for Forge.".into(),
        (SourceKind::ClaudeCode, _) => "Optional. Install Claude Code from claude.com/code to use a Claude subscription.".into(),
        (SourceKind::Codex, SourceState::Ready) => "Uses your ChatGPT plan through the codex command.".into(),
        (SourceKind::Codex, SourceState::NeedsLogin) => "Codex is installed but not signed in. Run `codex login` in another terminal, then press r.".into(),
        (SourceKind::Codex, _) => "Optional. Install Codex to use a ChatGPT plan through its command.".into(),
        (SourceKind::Antigravity, SourceState::Ready) => "Free Gemini access through the agy command.".into(),
        (SourceKind::Antigravity, SourceState::NeedsLogin) => "Antigravity is installed but not signed in. Run `agy` once to sign in, then press r.".into(),
        (SourceKind::Antigravity, _) => "Optional. Install Antigravity for free Gemini access.".into(),
        (SourceKind::CodexOauth, SourceState::Ready) => "Signed in to ChatGPT through Forge. No API key involved.".into(),
        (SourceKind::CodexOauth, _) => "Use your ChatGPT Plus or Pro plan with no API key. Enter opens your browser to sign in.".into(),
        (SourceKind::XaiOauth, SourceState::Ready) => "Signed in to xAI through Forge. No API key involved.".into(),
        (SourceKind::XaiOauth, _) => "Use your SuperGrok or X Premium plan with no API key. Enter opens your browser to sign in.".into(),
        (SourceKind::ApiKey, SourceState::Ready) => {
            if status == CardStatus::Rejected {
                "This key was refused. Enter lets you paste a new one.".into()
            } else {
                "A key is set. Space checks it again.".into()
            }
        }
        (SourceKind::ApiKey, _) if card.free_tier => format!(
            "{} has a free tier. Create a key at {url} (press o to open it), then paste it here. Forge checks it right away.",
            card.name
        ),
        (SourceKind::ApiKey, _) => format!(
            "Paid, billed by {}. Create a key at {url} (press o to open it), then paste it here.",
            card.name
        ),
        (SourceKind::Local, SourceState::Ready) => "Runs on this machine through Ollama. Free and private.".into(),
        (SourceKind::Local, _) => "Free and private. Enter installs Ollama if needed and downloads the model sized for this machine.".into(),
    }
}

pub(super) fn models(
    s: &Screen,
    width: usize,
    height: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
    let mut out = intro(
        "Connect a model",
        "Forge uses every model you connect and picks the best one for each task. You need at least one.",
        width,
    );
    let cards = s.cards();
    let cursor = s.cursor.min(cards.len().saturating_sub(1));
    let detail_rows = if s.entry.is_some() { 5 } else { 4 };
    let used = out.len() + detail_rows + 1;
    let list_h = height.saturating_sub(used).max(3);
    let first = if cursor < list_h {
        0
    } else {
        cursor + 1 - list_h
    };
    let name_w = 20.min(width / 3);
    let badge_w = 13;
    let mut focus_row = None;
    for (i, card) in cards.iter().enumerate().skip(first).take(list_h) {
        let status = s.card_status(card);
        let (glyph, glyph_style) = card_glyph(s, status);
        let (badge_text, badge_style) = badge(s, status, card);
        let detail_w = width.saturating_sub(2 + 2 + name_w + 1 + badge_w + 1);
        let selected = i == cursor;
        let spans = vec![
            Span::styled(format!("{glyph} "), glyph_style),
            Span::styled(
                padded(&card.name, name_w),
                if selected {
                    styles::fg().bold()
                } else {
                    styles::fg()
                },
            ),
            Span::styled(
                format!(" {}", padded(&card.detail, detail_w)),
                styles::muted(),
            ),
            Span::styled(format!(" {badge_text:>badge_w$}"), badge_style),
        ];
        if selected {
            focus_row = Some(out.len());
        }
        out.push(selectable(spans, selected, width));
    }
    out.push(blank());
    if let Some(entry) = &s.entry {
        out.push(heading(&entry.label));
        if let Some(hint) = &entry.hint {
            out.push(Line::from(Span::styled(
                truncate(hint, width),
                styles::muted(),
            )));
        }
        out.push(entry_field(s, width));
        out.push(Line::from(Span::styled(
            "Typing is hidden. The key goes to your keyring, nowhere else.",
            styles::faint(),
        )));
    } else if let Some(card) = cards.get(cursor) {
        let status = s.card_status(card);
        out.extend(
            paragraph(&source_help(card, status), width, 0, styles::muted())
                .into_iter()
                .take(4),
        );
    }
    (out, focus_row)
}

pub(crate) fn entry_field(s: &Screen, width: usize) -> Line<'static> {
    let buf_len = s.entry.as_ref().map_or(0, |e| e.buf.chars().count());
    Line::from(vec![
        Span::styled(format!("{} ", g().prompt), styles::brand()),
        Span::styled(
            mask(buf_len, width.saturating_sub(6).min(40)),
            styles::fg().bold(),
        ),
        Span::styled(g().caret.to_string(), styles::brand()),
    ])
}
