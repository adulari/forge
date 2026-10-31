//! The status line: engine and meters on one row. Configurable widgets and the built-in turn
//! readout sit on the left, the context gauge and mesh effort on the right. When the row is too
//! narrow, widgets drop in a fixed order (throughput, extras, tokens, mesh, effort, tier, cost,
//! then the model) and a faint `+N` says how many went. The gauge stays; everything dropped is
//! listed by `/status`.

use unicode_width::UnicodeWidthStr;

use super::span_fit::fit_spans;
use super::status_widgets::{context_gauge, fmt_cost, GaugeSize};
use super::*;
use crate::theme::glyphs::g;
use crate::theme::styles;

/// The order widgets are dropped in when the row is too narrow: first listed goes first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Throughput,
    Extras,
    Tokens,
    Mesh,
    Effort,
    Tier,
    Cost,
    Remote,
    Model,
}

const DROP_ORDER: [Class; 8] = [
    Class::Throughput,
    Class::Extras,
    Class::Tokens,
    Class::Mesh,
    Class::Effort,
    Class::Tier,
    Class::Cost,
    Class::Remote,
];

struct Item {
    class: Class,
    /// Joined to the previous item with no gap (`model · tier`, `model ⟨med⟩`).
    glued: bool,
    right: bool,
    spans: Vec<Span<'static>>,
}

impl Item {
    fn left(class: Class, spans: Vec<Span<'static>>) -> Self {
        Self {
            class,
            glued: false,
            right: false,
            spans,
        }
    }

    fn glued(class: Class, spans: Vec<Span<'static>>) -> Self {
        Self {
            glued: true,
            ..Self::left(class, spans)
        }
    }

    fn right(class: Class, spans: Vec<Span<'static>>) -> Self {
        Self {
            right: true,
            ..Self::left(class, spans)
        }
    }
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.content.width()).sum()
}

fn tokens_text(up: u64, down: u64) -> String {
    let glyphs = g();
    format!(
        "{}{} {}{}",
        glyphs.tok_up,
        human(up),
        glyphs.tok_down,
        human(down)
    )
}

/// The marker `model_effort_marker` gives a model whose provider chose the rung: nothing to show.
const AUTO_RUNG: &str = "⟨auto⟩";

fn is_cli_bridge(provider: &str) -> bool {
    matches!(provider, "claude-cli" | "codex-cli" | "agy-cli")
}

/// The tier chip text for the active routing, if any.
fn tier_text(app: &App) -> Option<String> {
    app.routing
        .as_ref()
        .map(|r| crate::app::lines::turn::plain_tier(&r.tier).to_string())
}

/// Items for one configured widget. A widget may yield several (the model carries its tier and
/// effort marker so each can drop on its own).
fn widget_items(widget: &forge_config::StatuslineWidget, app: &App, w: u16) -> Vec<Item> {
    use forge_config::StatuslineWidget as W;
    let glyphs = g();
    let muted = |text: String| vec![Span::styled(text, styles::muted())];
    match widget {
        W::Model => {
            let Some(routing) = app.routing.as_ref() else {
                return vec![Item::left(
                    Class::Model,
                    vec![Span::styled("no model yet", styles::faint())],
                )];
            };
            let mut head = Vec::new();
            let (mark, slot) = glyphs.provider(&routing.model);
            head.push(Span::styled(format!("{mark} "), styles::category(slot)));
            if w >= 100 {
                // A CLI bridge's name is plumbing: `haiku` already says which model answers.
                // A bare bridge id has no model to say, so its provider stays.
                if let Some((provider, model)) = routing.model.split_once("::") {
                    if model.is_empty() || !is_cli_bridge(provider) {
                        head.push(Span::styled(format!("{provider}::"), styles::faint()));
                    }
                }
            }
            head.push(Span::styled(
                model_short(Some(&routing.model)),
                styles::fg().bold(),
            ));
            // The routing tier is router vocabulary: the answer's label already says which
            // model took the turn, and `/status` keeps the tier for anyone who wants it.
            let mut items = vec![Item::left(Class::Model, head)];
            if let Some((marker, style)) =
                model_effort_marker(&routing.model, app.routed_effort.or(app.effort))
                    .filter(|(marker, _)| marker != AUTO_RUNG)
            {
                items.push(Item::glued(
                    Class::Effort,
                    vec![Span::raw(" "), Span::styled(marker, style)],
                ));
            }
            items
        }
        W::Tier => tier_text(app)
            .map(|t| vec![Item::left(Class::Tier, muted(t))])
            .unwrap_or_default(),
        W::SessionCost => {
            // Before any route exists there is no cost to speak of, and "untracked" would
            // describe a model nobody picked yet.
            if app.routing.is_none() && app.cost_usd <= 0.0 {
                return Vec::new();
            }
            let model_id = app.routing.as_ref().map(|r| r.model.as_str()).unwrap_or("");
            let text = cost_cell(model_id, app.cost_usd);
            // Billing vocabulary: `/status` and `/usage` still say how a model is paid for.
            if text == "untracked" || text == "subscription" {
                return Vec::new();
            }
            let (text, style) = if text.starts_with('$') {
                (fmt_cost(app.cost_usd), styles::ok())
            } else {
                (text, styles::muted())
            };
            vec![Item::left(Class::Cost, vec![Span::styled(text, style)])]
        }
        W::Effort => {
            // Auto routing is the default, and `/status` names it: a chip is for a choice made.
            if app.effort.is_none() {
                return Vec::new();
            }
            let (label, style) = mesh_effort_chip(app.effort, app.tick);
            vec![Item::right(Class::Mesh, vec![Span::styled(label, style)])]
        }
        // The mode lives on the composer border; `/status` lists it too.
        W::Mode => Vec::new(),
        W::TurnElapsed => {
            if !app.busy && !app.turn_ran {
                return Vec::new();
            }
            vec![Item::left(
                Class::Extras,
                muted(fmt_dur(app.turn_elapsed_secs)),
            )]
        }
        W::TokensIn => {
            if (!app.busy && !app.turn_ran) || app.turn_in == 0 {
                return Vec::new();
            }
            vec![Item::left(
                Class::Tokens,
                muted(format!("{}{}", glyphs.tok_up, human(app.turn_in))),
            )]
        }
        W::TokensOut => {
            if (!app.busy && !app.turn_ran) || app.turn_out == 0 {
                return Vec::new();
            }
            vec![Item::left(
                Class::Tokens,
                muted(format!("{}{}", glyphs.tok_down, human(app.turn_out))),
            )]
        }
        W::Throughput => app
            .throughput
            .display(app.busy, std::time::Instant::now())
            .map(|shown| {
                vec![Item::left(
                    Class::Throughput,
                    vec![Span::styled(shown.label(), throughput_style(&shown))],
                )]
            })
            .unwrap_or_default(),
        W::SessionTokens => {
            if app.session_in == 0 && app.session_out == 0 {
                return Vec::new();
            }
            let sigma = if glyphs.unicode { "Σ" } else { "total" };
            vec![Item::left(
                Class::Extras,
                muted(format!(
                    "{sigma} {}",
                    tokens_text(app.session_in, app.session_out)
                )),
            )]
        }
        W::GitBranch => app
            .git_branch
            .as_deref()
            .map(|b| {
                vec![Item::left(
                    Class::Extras,
                    muted(format!("{} {b}", glyphs.branch)),
                )]
            })
            .unwrap_or_default(),
        W::RepoName => app
            .repo_name
            .as_deref()
            .map(|r| vec![Item::left(Class::Extras, muted(r.to_string()))])
            .unwrap_or_default(),
        W::QuotaClaude => app
            .usage_overlay
            .claude_5h_pct
            .map(|pct| quota_item(format!("claude {pct:.0}%"), pct))
            .into_iter()
            .collect(),
        W::QuotaCodex => app
            .usage_overlay
            .codex_5h_pct
            .map(|pct| quota_item(format!("codex {pct:.0}%"), pct))
            .into_iter()
            .collect(),
        W::QuotaOpencodeGo => app
            .usage_overlay
            .opencode_go_tightest_window()
            .map(|(window, pct)| quota_item(format!("go {window} {pct:.0}%"), pct))
            .into_iter()
            .collect(),
        W::QuotaPace => {
            let Some(p) = app.quota_pace.as_ref() else {
                return Vec::new();
            };
            let style = if p.exhaustion_warning {
                styles::err()
            } else if p.projected_fraction_at_reset.unwrap_or(0.0) >= 0.70 {
                styles::warn()
            } else {
                styles::muted()
            };
            let short_window = match p.window.as_str() {
                "five_hour" => "5h",
                "weekly" => "wk",
                "monthly" => "mo",
                "" => "?",
                other => other,
            };
            let provider = p.provider.trim_end_matches("-cli");
            let text = match p.projected_fraction_at_reset {
                Some(proj) => format!("{provider} {short_window} -> {:.0}%", proj * 100.0),
                None => format!(
                    "{provider} {short_window} {:+.1}%/hr",
                    p.rate_per_hour * 100.0
                ),
            };
            vec![Item::left(Class::Extras, vec![Span::styled(text, style)])]
        }
        W::AnywhereSync => {
            let Some(badge) = app.anywhere_sync else {
                return Vec::new();
            };
            let (text, style) = if badge.error || badge.conflicts > 0 {
                ("sync !".to_string(), styles::warn())
            } else if badge.pending > 0 {
                (
                    format!("sync {}{}", badge.pending, glyphs.up),
                    styles::muted(),
                )
            } else {
                (format!("sync {}", glyphs.ok), styles::muted())
            };
            vec![Item::left(Class::Extras, vec![Span::styled(text, style)])]
        }
        W::McpStatus => {
            if app.mcp_count == 0 {
                return Vec::new();
            }
            vec![Item::left(
                Class::Extras,
                muted(format!("{} mcp", app.mcp_count)),
            )]
        }
        W::Custom {
            text,
            shell: Some(cmd),
            ..
        } => {
            let out = app
                .custom_widget_cache
                .get(cmd)
                .map(String::as_str)
                .unwrap_or(text);
            if out.is_empty() {
                return Vec::new();
            }
            vec![Item::left(Class::Extras, muted(out.to_string()))]
        }
        W::Custom {
            text, shell: None, ..
        } => {
            if text.is_empty() {
                return Vec::new();
            }
            vec![Item::left(Class::Extras, vec![Span::raw(text.clone())])]
        }
    }
}

fn quota_item(text: String, pct: f64) -> Item {
    let style = if pct >= 90.0 {
        styles::err()
    } else if pct >= 70.0 {
        styles::warn()
    } else {
        styles::muted()
    };
    Item::left(Class::Extras, vec![Span::styled(text, style)])
}

/// Every item the status row could show, in left-to-right order, before any width-driven drops.
fn collect_items(app: &App, w: u16) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for widget in &app.statusline_config.left {
        items.extend(widget_items(widget, app, w));
    }
    if app.statusline_config.auto_sync_badge() {
        items.extend(widget_items(
            &forge_config::StatuslineWidget::AnywhereSync,
            app,
            w,
        ));
    }
    let running = app.run_menu.running_count();
    if running > 0 {
        let label = match app.run_menu.sole_running_name() {
            Some(name) => name.to_string(),
            None => format!("{running} running"),
        };
        items.push(Item::left(
            Class::Extras,
            vec![
                Span::styled(g().live_dot, styles::ok()),
                Span::styled(format!(" {label}"), styles::muted()),
            ],
        ));
    }
    // The turn readout is built in, not a widget: tokens, then how fast. The tokens are a live
    // counter; once the turn ends its footer in the transcript carries the same numbers.
    if app.busy && app.turn_ran && (app.turn_in > 0 || app.turn_out > 0) {
        items.push(Item::left(
            Class::Tokens,
            vec![Span::styled(
                tokens_text(app.turn_in, app.turn_out),
                if app.busy {
                    styles::user()
                } else {
                    styles::muted()
                },
            )],
        ));
    }
    if let Some(shown) = app.throughput.display(app.busy, std::time::Instant::now()) {
        items.push(Item::left(
            Class::Throughput,
            vec![Span::styled(shown.label(), throughput_style(&shown))],
        ));
    }
    // Inline has no header, so the facts it would carry stay reachable here.
    if !app.fullscreen {
        if app.remote_active {
            items.push(Item::left(
                Class::Remote,
                vec![Span::styled(
                    format!("{} remote", g().live_dot),
                    styles::ok(),
                )],
            ));
        }
        if let (Some(location), false) = (
            &app.git_location,
            layout_shows_location(&app.statusline_config),
        ) {
            let branch = match &location.head {
                crate::git_location::GitHead::Branch(name) => name.clone(),
                crate::git_location::GitHead::Detached(_) => location.branch_label(),
            };
            items.push(Item::left(
                Class::Extras,
                vec![Span::styled(
                    format!(
                        "{} {}",
                        g().branch,
                        super::header::middle_truncate(&branch, 28)
                    ),
                    styles::muted(),
                )],
            ));
        }
    }
    items
}

fn throughput_style(shown: &crate::throughput::Throughput) -> Style {
    use crate::throughput::{FAST_TOK_PER_SEC, SLOW_TOK_PER_SEC};
    if !shown.live {
        return styles::muted();
    }
    if shown.tok_per_sec >= FAST_TOK_PER_SEC {
        styles::ok()
    } else if shown.tok_per_sec < SLOW_TOK_PER_SEC {
        styles::warn()
    } else {
        styles::user()
    }
}

fn layout_shows_location(config: &forge_config::StatuslineConfig) -> bool {
    use forge_config::StatuslineWidget as W;
    config
        .left
        .iter()
        .chain(config.extra_rows.iter().flatten())
        .any(|widget| matches!(widget, W::GitBranch | W::RepoName))
}

fn join_left(items: &[&Item], gap: &str) -> Vec<Span<'static>> {
    let mut out: Vec<Span<'static>> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 && !item.glued {
            out.push(Span::styled(gap.to_string(), styles::line()));
        }
        out.extend(item.spans.iter().cloned());
    }
    out
}

/// The gap between status items: two spaces, unless the user configured their own separator.
fn gap(app: &App) -> String {
    let configured = &app.statusline_config.separator;
    if configured == "  │  " {
        "  ".to_string()
    } else {
        configured.clone()
    }
}

/// What fits in `width` cells: the left spans, the right spans, how many widgets were dropped.
struct Fitted {
    left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    dropped: usize,
}

fn fit_row(app: &App, width: u16) -> Fitted {
    let gap = gap(app);
    let items = collect_items(app, width);
    let mut dropped_classes: Vec<Class> = Vec::new();
    let mut dropped = 0usize;
    let mut size = GaugeSize::Full;
    // The gauge is always there and never dropped; only its size gives way.
    let gauge = |size: GaugeSize| context_gauge(app.context_tokens, app.context_limit, size);
    let margin = 2usize; // one leading space, one trailing
    loop {
        let left_items: Vec<&Item> = items
            .iter()
            .filter(|i| !i.right && !dropped_classes.contains(&i.class))
            .collect();
        let right_items: Vec<&Item> = items
            .iter()
            .filter(|i| i.right && !dropped_classes.contains(&i.class))
            .collect();
        let mut left = join_left(&left_items, &gap);
        if dropped > 0 {
            left.push(Span::styled(format!(" +{dropped}"), styles::faint()));
        }
        let mut right: Vec<Span<'static>> = gauge(size);
        for item in &right_items {
            if !right.is_empty() {
                right.push(Span::raw("  "));
            }
            right.extend(item.spans.iter().cloned());
        }
        let need =
            spans_width(&left) + spans_width(&right) + margin + usize::from(!right.is_empty()) * 2;
        if need <= width as usize {
            return Fitted {
                left,
                right,
                dropped,
            };
        }
        // Drop the next droppable class that is actually present.
        let next = DROP_ORDER.iter().copied().find(|class| {
            !dropped_classes.contains(class) && items.iter().any(|i| i.class == *class)
        });
        if let Some(class) = next {
            dropped += items.iter().filter(|i| i.class == class).count();
            dropped_classes.push(class);
            continue;
        }
        if size == GaugeSize::Full {
            size = GaugeSize::Compact;
            continue;
        }
        if size == GaugeSize::Compact {
            size = GaugeSize::Minimal;
            continue;
        }
        let room = (width as usize)
            .saturating_sub(spans_width(&right) + margin + 2)
            .max(1);
        return Fitted {
            left: fit_spans(left, room),
            right,
            dropped,
        };
    }
}

/// Every configured widget rendered as a single span list (used by extra rows).
fn widget_spans(
    widget: &forge_config::StatuslineWidget,
    app: &App,
    w: u16,
) -> Option<Vec<Span<'static>>> {
    let items = widget_items(widget, app, w);
    if items.is_empty() {
        return None;
    }
    let mut spans = Vec::new();
    for item in items {
        spans.extend(item.spans);
    }
    Some(spans)
}

pub(crate) fn render_statusline(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let w = area.width;
    let row1 = Rect { height: 1, ..area };
    let fitted = fit_row(app, w);
    let mut left = vec![Span::raw(" ")];
    left.extend(fitted.left);
    let right_len = spans_width(&fitted.right) as u16;
    if right_len == 0 {
        frame.render_widget(Paragraph::new(TextLine::from(left)), row1);
    } else {
        let cols =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(right_len + 1)]).split(row1);
        frame.render_widget(Paragraph::new(TextLine::from(left)), cols[0]);
        frame.render_widget(
            Paragraph::new(TextLine::from(fitted.right)).alignment(Alignment::Right),
            cols[1],
        );
    }

    // Extra rows (user-configured): one more left-aligned row each, same widgets and separator.
    let sep = gap(app);
    let mut next_y = area.y + 1;
    for row_widgets in &app.statusline_config.extra_rows {
        if next_y >= area.y + area.height {
            break;
        }
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        let mut first = true;
        for widget in row_widgets {
            if let Some(w_spans) = widget_spans(widget, app, w) {
                if !first {
                    spans.push(Span::styled(sep.clone(), styles::line()));
                }
                first = false;
                spans.extend(w_spans);
            }
        }
        frame.render_widget(
            Paragraph::new(TextLine::from(spans)),
            Rect {
                y: next_y,
                height: 1,
                ..area
            },
        );
        next_y += 1;
    }

    // The user's `[statusline] command` output, last so the built-in row keeps its position.
    if let Some(out) = super::status_widgets::command_line(app) {
        if next_y < area.y + area.height {
            let mut spans = vec![Span::raw(" ")];
            spans.extend(crate::ansi_line::spans(out, styles::fg()));
            frame.render_widget(
                Paragraph::new(TextLine::from(spans)),
                Rect {
                    y: next_y,
                    height: 1,
                    ..area
                },
            );
        }
    }
}

/// How many widgets the status row dropped at `width`, for `/status` and tests.
pub(crate) fn dropped_widgets(app: &App, width: u16) -> usize {
    fit_row(app, width).dropped
}
