//! "Set up everything recommended": runs the decision-free actions one after another with a live
//! checklist, pauses on a browser sign-in until it finishes, and stops where a real choice is
//! needed.

use forge_onboarding::{recommended, Recommended, StepId, StepStatus, DEFAULT_ONLY_STEPS};

use super::screen::{Effect, NoticeKind, Purpose, Screen};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchStatus {
    Pending,
    Running,
    Done,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct BatchItem {
    pub rec: Recommended,
    pub status: BatchStatus,
}

#[derive(Debug, Clone, Default)]
pub struct Batch {
    pub items: Vec<BatchItem>,
}

impl Screen {
    /// Enter on the welcome page.
    pub(crate) fn start_batch(&mut self) -> Vec<Effect> {
        let items = recommended(
            &self.session.machine,
            &self.session.state,
            self.session.surface,
        );
        for step in DEFAULT_ONLY_STEPS {
            if self.session.state.status(step) == StepStatus::Pending
                && self.session.plan.iter().any(|p| p.id == step)
            {
                self.session.state.mark_done(step);
            }
        }
        self.session.state.mark_done(StepId::Welcome);
        self.batch = Some(Batch {
            items: items
                .into_iter()
                .map(|rec| BatchItem {
                    rec,
                    status: BatchStatus::Pending,
                })
                .collect(),
        });
        let mut effects = vec![Effect::Persist];
        effects.extend(self.batch_run_next());
        effects
    }

    /// Start the next runnable item; when none are left, settle the run.
    fn batch_run_next(&mut self) -> Vec<Effect> {
        let Some(batch) = self.batch.as_mut() else {
            return Vec::new();
        };
        let next = batch
            .items
            .iter()
            .position(|i| i.status == BatchStatus::Pending && i.rec.action.is_some());
        if let Some(index) = next {
            batch.items[index].status = BatchStatus::Running;
            let action = batch.items[index]
                .rec
                .action
                .clone()
                .expect("checked above");
            return vec![Effect::Run {
                action,
                purpose: Purpose::Batch(index),
            }];
        }
        self.batch_settle()
    }

    /// A batch job ended.
    pub(crate) fn batch_job_done(
        &mut self,
        index: usize,
        ok: bool,
        error: Option<String>,
    ) -> Vec<Effect> {
        if let Some(item) = self.batch.as_mut().and_then(|b| b.items.get_mut(index)) {
            item.status = if ok {
                BatchStatus::Done
            } else {
                BatchStatus::Failed(error.unwrap_or_else(|| "It did not work.".into()))
            };
            if ok {
                let step = item.rec.step;
                let siblings_done = self
                    .batch
                    .as_ref()
                    .map(|b| {
                        b.items
                            .iter()
                            .filter(|i| i.rec.step == step)
                            .all(|i| i.status == BatchStatus::Done)
                    })
                    .unwrap_or(true);
                if siblings_done && step != StepId::Models {
                    self.session.state.mark_done(step);
                }
            }
        }
        let mut effects = vec![Effect::Persist, Effect::Redetect];
        effects.extend(self.batch_run_next());
        effects
    }

    /// Every runnable item has run. Go where the person is needed, or to the summary.
    fn batch_settle(&mut self) -> Vec<Effect> {
        let batch = self.batch.take().unwrap_or_default();
        let failed: Vec<&BatchItem> = batch
            .items
            .iter()
            .filter(|i| matches!(i.status, BatchStatus::Failed(_)))
            .collect();
        let needs_choice = batch
            .items
            .iter()
            .find(|i| i.rec.action.is_none() && i.status == BatchStatus::Pending);
        let mut effects = vec![Effect::Persist, Effect::Redetect];
        if let Some(item) = needs_choice {
            self.notify(
                NoticeKind::Info,
                "Pick a model to go on. The rest is already set up.",
            );
            effects.extend(self.go_to_step(item.rec.step));
            return effects;
        }
        if let Some(item) = failed.first() {
            let reason = match &item.status {
                BatchStatus::Failed(why) => why.clone(),
                _ => String::new(),
            };
            self.notify(
                NoticeKind::Warn,
                format!("{} did not finish: {reason}", item.rec.title),
            );
            effects.extend(self.go_to_step(item.rec.step));
            return effects;
        }
        self.notify(NoticeKind::Ok, "Everything recommended is set up.");
        let last = self.session.plan.len() - 1;
        effects.extend(self.go_to(last));
        effects
    }
}
