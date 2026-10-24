//! Composer border content: the one live line, the state chips and the contextual hints.
//!
//! The composer's top border answers "what is happening now" on the left (spinner verb, elapsed,
//! tokens, or a waiting/stop/armed state) and "how my next message runs" on the right (queue, goal,
//! loop and mode chips). Its bottom border answers "what keys work now". Nothing here is drawn
//! anywhere else, so no fact appears twice.

use unicode_width::UnicodeWidthStr;

use super::span_fit::fit_spans;
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// Silence shorter than this is normal and not worth a word.
const QUIET_NOTICE_SECS: u64 = 15;
/// Silence at or past this reads as a stall, not as hidden reasoning.
const QUIET_WARN_SECS: u64 = 60;

/// One entry of the hint row: a key and what it does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hint {
    pub key: String,
    pub label: &'static str,
}

/// A hint, with arrows in the key swapped for their ASCII twins when the glyph tier needs it.
pub(crate) fn hint(key: &str, label: &'static str) -> Hint {
    Hint {
        key: key_text(key),
        label,
    }
}

fn key_text(key: &str) -> String {
    if g().unicode {
        return key.to_string();
    }
    key.replace('↑', "^")
        .replace('↓', "v")
        .replace('←', "<")
        .replace('→', ">")
}

#[derive(Debug, Clone)]
pub(crate) struct LiveState {
    pub spans: Vec<Span<'static>>,
}

fn capitalised(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The verb and a plain-English detail for the turn's current phase.
fn verb_and_detail(app: &App) -> (String, Option<String>) {
    let activity = &app.turn_activity;
    let tool_target = |detail: &str| {
        detail
            .strip_prefix("executing ")
            .map(|name| tool_display_name(name).to_string())
            .filter(|name| !name.is_empty())
    };
    match activity.phase {
        TurnPhase::Preparing => ("Preparing".into(), Some("picking a route".into())),
        TurnPhase::ContactingModel => (
            "Routing".into(),
            app.routing.as_ref().map(|r| model_short(Some(&r.model))),
        ),
        TurnPhase::ReceivingProvider => {
            if let Some((name, _)) = &activity.composing_tool {
                (format!("Writing {name} call"), None)
            } else if activity.thinking_tokens > 0 {
                (
                    "Thinking".into(),
                    Some(format!("~{} tokens", human(activity.thinking_tokens))),
                )
            } else {
                ("Waiting".into(), Some("for the model".into()))
            }
        }
        TurnPhase::Reasoning => ("Thinking".into(), None),
        TurnPhase::Responding => ("Writing".into(), None),
        // Full-screen shows the running call in the transcript, target and all; the border only
        // says what kind of work it is, so the command is not printed twice.
        TurnPhase::RunningTool if app.fullscreen => (
            running_phrase(tool_target(&activity.detail).as_deref()),
            None,
        ),
        TurnPhase::RunningTool => (
            match tool_target(&activity.detail) {
                Some(name) => format!("Running {name}"),
                None => "Running tool".into(),
            },
            None,
        ),
        TurnPhase::ProcessingToolResult => ("Thinking".into(), Some("reading the result".into())),
        TurnPhase::Auxiliary => ("Working".into(), Some(activity.detail.clone())),
        TurnPhase::Recovering => match &app.model_search {
            Some((_, true)) => ("Retrying".into(), Some("same model".into())),
            _ => ("Finding a model".into(), None),
        },
        TurnPhase::Coordinating => ("Coordinating".into(), None),
        TurnPhase::Compacting => ("Compacting".into(), None),
        TurnPhase::Finalizing => ("Finishing".into(), None),
    }
}

/// `Running a command` for `Run cargo test`: the work, from the verb alone.
fn running_phrase(target: Option<&str>) -> String {
    let verb = target
        .and_then(|t| t.split_whitespace().next())
        .unwrap_or("");
    match verb {
        "Run" => "Running a command",
        "Read" => "Reading",
        "Write" => "Writing a file",
        "Edit" | "Patch" => "Editing",
        "Search" | "Find" => "Searching",
        "List" => "Listing",
        "Fetch" => "Fetching",
        "Web" => "Searching the web",
        "Agents" => "Running agents",
        "Script" => "Running a script",
        "" => "Running tool",
        other => return format!("Running {other}"),
    }
    .to_string()
}

fn busy_line(app: &App) -> Vec<Span<'static>> {
    let glyphs = g();
    let sep = format!(" {} ", glyphs.sep);
    let (verb, detail) = verb_and_detail(app);
    let mut spans = vec![
        Span::styled(glyphs.spin(app.tick).to_string(), styles::spinner(app.tick)),
        Span::styled(format!(" {verb}"), styles::fg().bold()),
    ];
    let mut push = |text: String, style: Style| {
        spans.push(Span::styled(sep.clone(), styles::faint()));
        spans.push(Span::styled(text, style));
    };
    if let Some(detail) = detail.filter(|d| !d.is_empty()) {
        push(detail, styles::muted());
    }
    push(fmt_dur(app.turn_elapsed_secs), styles::muted());
    let streamed = if app.turn_out > 0 {
        Some(format!("{}{} tok", glyphs.tok_down, human(app.turn_out)))
    } else if app.turn_activity.response_chars > 0 {
        Some(format!(
            "~{}{} tok",
            glyphs.tok_down,
            human((app.turn_activity.response_chars / 4) as u64)
        ))
    } else {
        None
    };
    if let Some(text) = streamed {
        push(text, styles::muted());
    }
    if let Some(rate) = app.throughput.display(true, std::time::Instant::now()) {
        push(rate.label(), styles::muted());
    }
    let quiet = app.turn_quiet_secs();
    if quiet >= QUIET_NOTICE_SECS {
        let style = if quiet >= QUIET_WARN_SECS {
            styles::warn()
        } else {
            styles::muted()
        };
        push(format!("last event {} ago", fmt_dur(quiet)), style);
    }
    spans
}

/// A stop reason as `(is_error, reason, recovery, short, minimal)`: the full wording, a single
/// short phrase, and the bare recovery, for borders that get narrower and narrower.
fn stop_line(
    reason: forge_types::StopReason,
) -> Option<(bool, &'static str, &'static str, &'static str, &'static str)> {
    use forge_types::StopReason as R;
    match reason {
        R::MaxSteps => Some((
            false,
            "Step limit reached",
            "send `continue` to resume",
            "Step limit · send continue",
            "send continue",
        )),
        R::BudgetExhausted => Some((
            true,
            "Budget cap reached",
            "raise it in /config",
            "Budget cap reached",
            "Budget cap",
        )),
        R::TasksUnfinished => Some((
            true,
            "Stopped with tasks unfinished",
            "Ctrl+O shows them",
            "Tasks unfinished",
            "Tasks left",
        )),
        R::Stalled => Some((
            false,
            "Paused: no progress",
            "send `continue` or say what to try",
            "No progress · send continue",
            "send continue",
        )),
        R::NoOutput => Some((
            true,
            "No output, the turn failed",
            "send the message again",
            "No output",
            "No output",
        )),
        _ => None,
    }
}

fn model_unavailable(app: &App) -> bool {
    app.done
        && app.transcript_ui.model_unavailable
        && app.last_stop_reason == Some(forge_types::StopReason::NoOutput)
}

/// The composer's live line: loading, an armed key, waiting for you, the working verb line, or a
/// stop reason. `None` when idle with nothing to say.
pub(crate) fn live_state(app: &App, room: usize) -> Option<LiveState> {
    let glyphs = g();
    if let Some(label) = app.loading {
        return Some(LiveState {
            spans: vec![
                Span::styled(glyphs.spin(app.tick).to_string(), styles::spinner(app.tick)),
                Span::styled(format!(" {}", capitalised(label)), styles::fg().bold()),
            ],
        });
    }
    if app.quit_hint_active() {
        return Some(LiveState {
            spans: vec![Span::styled(
                format!("{} {} again to quit", glyphs.warn, app.quit_chord()),
                styles::warn(),
            )],
        });
    }
    if app.esc_hint_active() {
        return Some(LiveState {
            spans: vec![Span::styled(
                format!("{} Esc again to rewind", glyphs.warn),
                styles::warn(),
            )],
        });
    }
    if app.prompt.is_some() || app.awaiting_question() {
        let what = if app.awaiting_question() {
            "your answer"
        } else {
            "your approval"
        };
        return Some(LiveState {
            spans: vec![
                Span::styled(
                    format!("{} Waiting for you", glyphs.live_dot),
                    styles::warn(),
                ),
                Span::styled(format!(" {} {what}", glyphs.sep), styles::muted()),
            ],
        });
    }
    if let Some((_, retrying)) = &app.model_search {
        let label = if *retrying {
            "Retrying model"
        } else {
            "Finding a model"
        };
        return Some(LiveState {
            spans: vec![
                Span::styled(glyphs.spin(app.tick).to_string(), styles::spinner(app.tick)),
                Span::styled(format!(" {label}"), styles::warn()),
            ],
        });
    }
    if app.busy {
        return Some(LiveState {
            spans: busy_line(app),
        });
    }
    if app.done {
        // The turn error above already says the model is unavailable, and the border hint
        // points at /model: a third statement here would only repeat them.
        let stop = if model_unavailable(app) {
            None
        } else {
            app.last_stop_reason.and_then(stop_line)
        };
        if let Some((is_error, reason, recovery, short, minimal)) = stop {
            let (glyph, style) = if is_error {
                (glyphs.fail, styles::err())
            } else {
                (glyphs.warn, styles::warn())
            };
            let full_w = glyph.width() + 1 + reason.width() + 3 + recovery.width();
            let spans = if full_w <= room {
                vec![
                    Span::styled(format!("{glyph} {reason}"), style),
                    Span::styled(format!(" {} {recovery}", glyphs.sep), styles::muted()),
                ]
            } else if glyph.width() + 1 + short.width() <= room {
                vec![Span::styled(format!("{glyph} {short}"), style)]
            } else {
                vec![Span::styled(format!("{glyph} {minimal}"), style)]
            };
            return Some(LiveState { spans });
        }
    }
    None
}

/// The chips on the composer's top-right edge, left to right, the mode chip last. The caller
/// drops chips from the left when the border is narrow.
pub(crate) fn chips(app: &App) -> Vec<Vec<Span<'static>>> {
    let glyphs = g();
    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    if app.input.contains('\n') {
        let lines = app.input.split('\n').count();
        out.push(vec![Span::styled(
            format!("{lines} lines"),
            styles::faint(),
        )]);
    }
    if let Some(loop_chip) = &app.loop_chip {
        out.push(vec![Span::styled(
            format!("{} {loop_chip}", glyphs.loop_),
            styles::muted(),
        )]);
    }
    if let Some(goal_chip) = &app.goal_chip {
        out.push(vec![Span::styled(
            format!("{} {goal_chip}", glyphs.goal),
            styles::muted(),
        )]);
    }
    if !app.temper.is_empty() {
        out.push(vec![Span::styled(
            app.temper.clone(),
            styles::temper(&app.temper),
        )]);
    }
    out
}

/// Keep at most five hints; when the row is too narrow drop from the second-to-last backwards,
/// so the first (most useful) and the last (`? keys`) survive longest.
pub(crate) fn fit_hints(mut hints: Vec<Hint>, budget: usize) -> Vec<Hint> {
    let cost = |hints: &[Hint]| -> usize {
        let sep = 3; // " · "
        hints
            .iter()
            .map(|h| h.key.width() + 1 + h.label.width())
            .sum::<usize>()
            + sep * hints.len().saturating_sub(1)
    };
    hints.truncate(5);
    while hints.len() > 2 && cost(&hints) > budget {
        hints.remove(hints.len() - 2);
    }
    while hints.len() > 1 && cost(&hints) > budget {
        hints.remove(0);
    }
    if cost(&hints) > budget {
        hints.clear();
    }
    hints
}

fn default_hints(app: &App) -> Vec<Hint> {
    if app.quit_hint_active() {
        return vec![
            hint(app.quit_chord(), "quit"),
            hint("any key", "cancels"),
            hint("?", "keys"),
        ];
    }
    if app.esc_hint_active() {
        return vec![hint("Esc", "again to rewind"), hint("?", "keys")];
    }
    if app.busy {
        return vec![
            hint("Esc", "stop"),
            hint("Enter", "queue"),
            hint("Ctrl+↑", "stronger model"),
            hint("Ctrl+O", "details"),
            hint("?", "keys"),
        ];
    }
    if model_unavailable(app) {
        return vec![
            hint("/model", "pick another"),
            hint("Esc Esc", "rewind"),
            hint("?", "keys"),
        ];
    }
    if app.done && app.last_stop_reason.and_then(stop_line).is_some() {
        return vec![
            hint("continue", "resume"),
            hint("Ctrl+O", "tasks"),
            hint("Esc Esc", "rewind"),
            hint("?", "keys"),
        ];
    }
    if crate::bang_command(&app.input).is_some() {
        return vec![
            hint("Enter", "run in shell"),
            hint("Ctrl+U", "clear"),
            hint("?", "keys"),
        ];
    }
    if !app.input.is_empty() {
        return vec![
            hint("Enter", "send"),
            hint("Ctrl+J", "newline"),
            hint("@", "files"),
            hint("Shift+Tab", "mode"),
            hint("?", "keys"),
        ];
    }
    if app.suggested_prompt.is_some() {
        return vec![
            hint("Tab", "accept"),
            hint("Esc Esc", "rewind"),
            hint("Ctrl+T", "last tool"),
            hint("/", "commands"),
            hint("?", "keys"),
        ];
    }
    vec![
        hint("/", "commands"),
        hint("@", "files"),
        hint(&app.action_chord("command_center"), "actions"),
        hint("Shift+Tab", "mode"),
        hint("?", "keys"),
    ]
}

/// The hint row for the current state. A decision block or popup that owns the keyboard supplies
/// its own set through its `hints` hook; otherwise the composer's own state decides.
pub(crate) fn hints(app: &App) -> Vec<Hint> {
    super::permission::hints(app)
        .or_else(|| super::popups::hints(app))
        .or_else(|| super::panels::hints(app))
        .unwrap_or_else(|| default_hints(app))
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

/// How long the border pulses while the app waits for you.
const PULSE_SECS: u64 = 5;

/// The border colour: while the app waits for you it alternates warn and muted once a second for
/// the first five seconds (a held decision should catch the eye), then stays warn; otherwise it
/// is the mode colour. Reduced motion never pulses.
pub(crate) fn border_style(app: &App) -> Style {
    if app.prompt.is_some() || app.awaiting_question() {
        let since = app.waiting_since.get().unwrap_or_else(|| {
            app.waiting_since.set(Some(app.tick));
            app.tick
        });
        // The render loop advances `tick` about every 60 ms while a turn runs.
        let secs = (app.tick.saturating_sub(since) as u64).saturating_mul(60) / 1_000;
        if crate::theme::t().animates() && secs < PULSE_SECS && secs % 2 == 1 {
            return styles::muted();
        }
        return styles::warn();
    }
    app.waiting_since.set(None);
    if app.temper.is_empty() {
        styles::line()
    } else {
        styles::temper(&app.temper)
    }
}

/// The composer frame with its three border zones filled in for `width` cells.
pub(crate) fn composer_block(app: &App, width: u16) -> Block<'static> {
    let glyphs = g();
    let avail = (width as usize).saturating_sub(2);
    let border_style = border_style(app);

    // Right edge first: the chips are the part that must never collide with the live line.
    let mut chips = chips(app);
    let join = |chips: &[Vec<Span<'static>>]| -> Vec<Span<'static>> {
        let mut spans = vec![Span::raw(" ")];
        for (i, chip) in chips.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(format!(" {} ", glyphs.sep), styles::faint()));
            }
            spans.extend(chip.iter().cloned());
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            glyphs.border.horizontal_top.to_string(),
            border_style,
        ));
        spans
    };
    while chips.len() > 1 && spans_width(&join(&chips)) > avail / 2 {
        chips.remove(0);
    }
    let right = if chips.is_empty() {
        Vec::new()
    } else {
        join(&chips)
    };
    let right_w = spans_width(&right);
    // The live line gets what the chips leave: a leading `─ `, a trailing space, one spare cell.
    let live = live_state(app, avail.saturating_sub(right_w + 3));

    let mut block = Block::bordered()
        .border_set(glyphs.border)
        .border_style(border_style)
        .padding(Padding::horizontal(1));
    if let Some(live) = live {
        let mut spans = vec![
            Span::styled(glyphs.border.horizontal_top.to_string(), border_style),
            Span::raw(" "),
        ];
        spans.extend(live.spans);
        let room = avail.saturating_sub(right_w + 2);
        let mut fitted = fit_spans(spans, room);
        fitted.push(Span::raw(" "));
        block = block.title(TextLine::from(fitted));
    }
    if !right.is_empty() {
        block = block.title(TextLine::from(right).right_aligned());
    }

    let hints = fit_hints(hints(app), avail.saturating_sub(4));
    if !hints.is_empty() {
        let mut spans = vec![Span::raw(" ")];
        for (i, h) in hints.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(format!(" {} ", glyphs.sep), styles::faint()));
            }
            spans.extend(styles::hint_spans(&h.key, h.label));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            glyphs.border.horizontal_bottom.to_string(),
            border_style,
        ));
        block = block.title_bottom(TextLine::from(spans).right_aligned());
    }
    block
}

impl App {
    /// Refresh the goal and loop chips from the autonomous runs' iteration counts (`None` when
    /// that run is not active). The goal chip shows task progress once a plan exists.
    pub fn set_autonomy_chips(&mut self, goal_iter: Option<usize>, loop_iter: Option<usize>) {
        self.goal_chip = goal_iter.map(|iter| {
            if self.tasks.is_empty() {
                format!("iteration {iter}")
            } else {
                let done = self
                    .tasks
                    .iter()
                    .filter(|t| t.status == forge_types::TodoStatus::Done)
                    .count();
                format!("{done}/{}", self.tasks.len())
            }
        });
        self.loop_chip = loop_iter.map(|iter| format!("iteration {iter}"));
    }
}

/// The live line as plain text, for assertions.
#[cfg(test)]
pub(crate) fn live_text(app: &App) -> String {
    live_state(app, 200)
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .unwrap_or_default()
}

/// The hint row as plain text (`key label · key label`), for assertions.
#[cfg(test)]
pub(crate) fn hint_text(app: &App) -> String {
    hints(app)
        .iter()
        .map(|h| format!("{} {}", h.key, h.label))
        .collect::<Vec<_>>()
        .join(" · ")
}
