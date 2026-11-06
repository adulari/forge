//! `/ship`: one action from a finished session to a watched pull request. Data and key handling
//! only; the binary performs `POST /api/sessions/{id}/ship` and hands the outcome back, and
//! [`render`] draws the card. The rows mirror the phone and desktop sheet: committed, pushed,
//! PR opened, watching, stopping at the first refusal with its reason.

use crate::app::KeyKind;

mod render;
pub(crate) use render::render_ship_menu;

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Done,
    Skipped,
    Refused,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShipStepLine {
    pub step: String,
    pub status: StepStatus,
    pub detail: String,
}

/// The files a ship would commit, as the daemon's preview names them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShipPlan {
    /// `index` | `worktree` | `session`
    pub source: String,
    /// `(path, kind)`; kind is created | modified | deleted | renamed.
    pub files: Vec<(String, String)>,
    /// Dirty files that stay out of the commit.
    pub left_out: Vec<String>,
}

impl ShipPlan {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        if !value.is_object() {
            return None;
        }
        let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
        Some(Self {
            source: text(&value["source"]),
            files: value["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|f| (text(&f["path"]), text(&f["kind"])))
                .collect(),
            left_out: value["left_out"]
                .as_array()
                .into_iter()
                .flatten()
                .map(text)
                .collect(),
        })
    }

    /// What the commit is drawn from, in a few words.
    pub fn source_label(&self) -> &'static str {
        match self.source.as_str() {
            "index" => "the files you staged",
            "session" => "this session's files",
            _ => "all changes in the worktree",
        }
    }

    /// The exact commit set: a heading, one line per file (at most `max_files`), then the
    /// files left out.
    pub fn lines(&self, max_files: usize) -> Vec<String> {
        if self.files.is_empty() {
            return vec!["No file changes: the commits already on the branch ship.".to_string()];
        }
        let mut lines = vec![format!(
            "Commits {} file(s) from {}:",
            self.files.len(),
            self.source_label()
        )];
        for (path, kind) in self.files.iter().take(max_files) {
            let mark = match kind.as_str() {
                "created" => 'A',
                "deleted" => 'D',
                "renamed" => 'R',
                _ => 'M',
            };
            lines.push(format!("  {mark} {path}"));
        }
        if self.files.len() > max_files {
            lines.push(format!("  +{} more", self.files.len() - max_files));
        }
        if !self.left_out.is_empty() {
            let shown: Vec<&str> = self.left_out.iter().take(3).map(String::as_str).collect();
            let more = self.left_out.len().saturating_sub(shown.len());
            lines.push(format!(
                "Left out: {}{}",
                shown.join(", "),
                if more > 0 {
                    format!(" +{more} more")
                } else {
                    String::new()
                }
            ));
        }
        lines
    }
}

/// The daemon's answer, reduced to what a terminal shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShipOutcome {
    pub ok: bool,
    /// The answer to a `preview` request: nothing was changed.
    pub preview: bool,
    pub plan: Option<ShipPlan>,
    pub steps: Vec<ShipStepLine>,
    /// `(step, reason)` of the refusal that stopped the run.
    pub refused: Option<(String, String)>,
    pub pr_number: Option<u64>,
    pub pr_url: Option<String>,
}

impl ShipOutcome {
    /// Read the daemon's `POST /api/sessions/{id}/ship` answer.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let text = |v: &serde_json::Value| v.as_str().unwrap_or_default().to_string();
        let steps = value["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| ShipStepLine {
                step: text(&s["step"]),
                status: match s["status"].as_str() {
                    Some("refused") => StepStatus::Refused,
                    Some("skipped") => StepStatus::Skipped,
                    _ => StepStatus::Done,
                },
                detail: text(&s["detail"]),
            })
            .collect();
        let refused = value["refused"].is_object().then(|| {
            (
                text(&value["refused"]["step"]),
                text(&value["refused"]["reason"]),
            )
        });
        Self {
            ok: value["ok"].as_bool().unwrap_or(false),
            preview: value["preview"].as_bool().unwrap_or(false),
            plan: ShipPlan::from_json(&value["plan"]),
            steps,
            refused,
            pr_number: value["pr"]["number"].as_u64(),
            pr_url: value["pr"]["url"].as_str().map(str::to_string),
        }
    }

    /// Plain lines for a terminal or a toast: one per step, then the headline.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = self
            .steps
            .iter()
            .map(|s| {
                let mark = match s.status {
                    StepStatus::Done => "ok     ",
                    StepStatus::Skipped => "skipped",
                    StepStatus::Refused => "STOPPED",
                };
                format!("{mark} {:<6} {}", s.step, s.detail)
            })
            .collect();
        lines.push(headline(Some(self), false));
        lines
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    Pending,
    Active,
    Done,
    Skipped,
    Refused,
    /// Never reached because an earlier step refused.
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShipRow {
    pub label: &'static str,
    pub state: RowState,
    pub detail: String,
}

const ROWS: [(&str, &str); 4] = [
    ("commit", "Commit changes"),
    ("push", "Push branch"),
    ("pr", "Open pull request"),
    ("watch", "Watch and merge"),
];

pub fn rows(outcome: Option<&ShipOutcome>, running: bool) -> Vec<ShipRow> {
    ROWS.iter()
        .map(|(key, label)| {
            let (state, detail) = match outcome {
                None => (
                    if running && *key == "commit" {
                        RowState::Active
                    } else {
                        RowState::Pending
                    },
                    String::new(),
                ),
                Some(o) => match o.steps.iter().find(|s| s.step == *key) {
                    Some(s) => (
                        match s.status {
                            StepStatus::Done => RowState::Done,
                            StepStatus::Skipped => RowState::Skipped,
                            StepStatus::Refused => RowState::Refused,
                        },
                        s.detail.clone(),
                    ),
                    None if o.refused.is_some() => (RowState::Blocked, String::new()),
                    None => (RowState::Pending, String::new()),
                },
            };
            ShipRow {
                label,
                state,
                detail,
            }
        })
        .collect()
}

pub fn headline(outcome: Option<&ShipOutcome>, running: bool) -> String {
    if running {
        return "Shipping…".to_string();
    }
    let Some(o) = outcome else {
        return "Commit, push, open the PR and let Forge merge it when checks pass.".to_string();
    };
    if o.ok {
        let pr = o
            .pr_number
            .map_or_else(|| "The pull request".to_string(), |n| format!("PR #{n}"));
        return format!("{pr} is open and being watched. Forge merges it when checks pass.");
    }
    match &o.refused {
        Some((step, reason)) if step == "branch" => format!("Stopped before committing: {reason}"),
        Some((step, reason)) => {
            let label = ROWS
                .iter()
                .find(|(k, _)| k == step)
                .map_or("a step", |(_, l)| *l);
            format!("Stopped at {}: {reason}", label.to_lowercase())
        }
        None => "Shipping stopped.".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShipAction {
    None,
    Close,
    Start,
}

#[derive(Debug, Clone, Default)]
pub struct ShipMenu {
    pub open: bool,
    pub running: bool,
    pub outcome: Option<ShipOutcome>,
    pub error: Option<String>,
    pub tick: u32,
    /// Waiting for the daemon to say what the commit would hold.
    pub previewing: bool,
    /// Set by [`ShipMenu::open`] and retries; the driver sends the preview request and clears it.
    pub needs_preview: bool,
    /// The previewed commit set; Enter ships it.
    pub plan: Option<ShipPlan>,
}

impl ShipMenu {
    pub fn open(&mut self) {
        *self = Self {
            open: true,
            previewing: true,
            needs_preview: true,
            ..Self::default()
        };
    }

    /// The daemon's answer to the preview request.
    pub fn finish_preview(&mut self, result: Result<ShipOutcome, String>) {
        self.previewing = false;
        match result {
            Ok(outcome) if outcome.ok => {
                self.error = None;
                self.outcome = None;
                self.plan = Some(outcome.plan.unwrap_or_default());
            }
            Ok(outcome) => {
                self.plan = None;
                self.outcome = Some(outcome);
            }
            Err(error) => {
                self.plan = None;
                self.error = Some(error);
            }
        }
    }

    /// The preview is in and nothing has been shipped: Enter ships exactly this set.
    pub fn confirmable(&self) -> bool {
        self.plan.is_some() && self.outcome.is_none() && self.error.is_none()
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn finish(&mut self, result: Result<ShipOutcome, String>) {
        self.running = false;
        match result {
            Ok(outcome) => {
                self.error = None;
                self.outcome = Some(outcome);
            }
            Err(error) => {
                self.outcome = None;
                self.error = Some(error);
            }
        }
    }

    /// Done once the run succeeded; a refusal or an error can be retried.
    pub fn succeeded(&self) -> bool {
        self.outcome.as_ref().is_some_and(|o| o.ok)
    }

    pub fn key(&mut self, key: KeyKind) -> ShipAction {
        self.tick = self.tick.wrapping_add(1);
        if self.running {
            return ShipAction::None;
        }
        if self.previewing && !matches!(key, KeyKind::Esc | KeyKind::Interrupt) {
            return ShipAction::None;
        }
        match key {
            KeyKind::Esc | KeyKind::Interrupt => {
                self.close();
                ShipAction::Close
            }
            KeyKind::Enter if self.succeeded() => {
                self.close();
                ShipAction::Close
            }
            KeyKind::Enter | KeyKind::Char('r') if self.confirmable() => {
                self.running = true;
                ShipAction::Start
            }
            KeyKind::Enter | KeyKind::Char('r') => {
                *self = Self {
                    open: true,
                    previewing: true,
                    needs_preview: true,
                    tick: self.tick,
                    ..Self::default()
                };
                ShipAction::None
            }
            _ => ShipAction::None,
        }
    }
}
