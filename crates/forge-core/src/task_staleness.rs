//! A task the model can no longer resolve must not re-drive the session forever — and the harness
//! must never quietly delete one either.
//!
//! Live failure (2026-09-09, session `07ca114e`): an auto-compaction left one task — "Strip
//! live-reuse, keep perf wins" — `InProgress`, with the reasoning that produced it no longer in
//! context. From then on every turn ended with the completion gate seeing an unfinished task, and
//! every continue-nudge spent its steps re-reading the same files to work out what the task meant.
//! Those reads count as progress in [`crate::nudge_policy`], so the nudge budget never ran out;
//! the session looped until a human interrupted it and made the call by hand.
//!
//! The missing signal is structural and cheap: a task whose title and status have not moved for
//! several turns — turns that changed nothing in the workspace either — is not being worked on,
//! whatever the model says about it. This module counts that and escalates twice: first it makes
//! the model decide about the task by name, then, if nothing moves after that either, it asks the
//! user.
//!
//! The second step used to remove the task. That was wrong for real work (2026-09-13, the same
//! session): the counter advanced on every user message, the user steered every ~2 minutes, and
//! eight tasks were deleted in one day — each during shell-driven debugging that WAS the task, and
//! several at the exact moment a steering message arrived. Only the user can say what an old task
//! still means, so the harness asks; with nobody present to answer, the task stays.
//!
//! The same turn-start pass also shows the model the list a new message lands on — see
//! [`carried_list_note`].

use forge_types::{PresenterEvent, QChoice, TodoItem, TodoStatus};
use std::collections::HashMap;

/// Turns an unfinished task may sit unchanged before the harness forces a decision about it.
///
/// Three, not one: a task legitimately spans several turns of real work, and the status does not
/// have to change while it does. What is not legitimate is three consecutive turns in which
/// nothing about it moved at all.
pub(crate) const ESCALATE_AFTER_TURNS: u32 = 3;

/// Further unchanged turns after the escalation before the harness asks the user about the task.
pub(crate) const ASK_AFTER_ESCALATION: u32 = 2;

/// Titles quoted back in a carried-list note before the rest are summarized as a count.
const LISTED_TASKS: usize = 12;

pub(crate) const KEEP: &str = "Keep working on it";
pub(crate) const DONE: &str = "It is done";
pub(crate) const REMOVE: &str = "Remove it";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    status: TodoStatus,
    /// Consecutive still turns this exact (title, status) pair has survived.
    turns: u32,
    /// The transcript was compacted while this task was open, so the detail behind it is gone.
    compacted: bool,
    /// The model has already been told to resolve or drop this one.
    escalated: bool,
}

/// What the harness should do about the stalled tasks in the current list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Verdict {
    /// Tasks the model must resolve or drop in its next step.
    pub escalate: Vec<String>,
    /// Tasks the model was already told about and still did not move: the user decides.
    pub ask: Vec<String>,
    /// Any of them were open across a compaction.
    pub compacted: bool,
}

/// Counts how long each unfinished task has stood still. One instance per session.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tracker {
    open: HashMap<String, Entry>,
    /// The turn just run changed the workspace.
    worked: bool,
}

impl Tracker {
    /// Record that the transcript was compacted: every task open right now loses the context that
    /// explains it, which is exactly the case that produced the live loop.
    pub(crate) fn note_compaction(&mut self) {
        for entry in self.open.values_mut() {
            entry.compacted = true;
        }
    }

    /// Record the state-changing tool calls a model loop made (a write, or a shell command on the
    /// direct path). Open tasks do not age across a turn that did such work: the work may well be
    /// the task, and a status field left untouched while it happens is not evidence of neglect.
    pub(crate) fn note_work(&mut self, mutations: u64) {
        self.worked |= mutations > 0;
    }

    /// Advance one turn over the session's current task list, returning what to do about anything
    /// that has stopped moving. Tasks that are Done, renamed, re-statused, or gone from the list
    /// reset — only a genuinely untouched one accumulates turns.
    pub(crate) fn turn(&mut self, tasks: &[TodoItem]) -> Option<Verdict> {
        let worked = std::mem::take(&mut self.worked);
        let mut next: HashMap<String, Entry> = HashMap::new();
        let mut escalate = Vec::new();
        let mut ask = Vec::new();
        let mut compacted = false;

        for task in tasks.iter().filter(|t| t.status != TodoStatus::Done) {
            let mut entry = match self.open.remove(&task.title) {
                Some(prev) if prev.status == task.status => Entry {
                    turns: if worked {
                        prev.turns
                    } else {
                        prev.turns.saturating_add(1)
                    },
                    ..prev
                },
                _ => Entry {
                    status: task.status,
                    turns: 1,
                    compacted: false,
                    escalated: false,
                },
            };
            if entry.escalated && entry.turns >= ESCALATE_AFTER_TURNS + ASK_AFTER_ESCALATION {
                compacted |= entry.compacted;
                ask.push(task.title.clone());
                // Whatever the answer, the task starts over, so the user is not asked about it
                // again on the very next turn.
                entry = Entry {
                    status: task.status,
                    turns: 0,
                    compacted: false,
                    escalated: false,
                };
            } else if !entry.escalated && entry.turns >= ESCALATE_AFTER_TURNS {
                entry.escalated = true;
                compacted |= entry.compacted;
                escalate.push(task.title.clone());
            }
            next.insert(task.title.clone(), entry);
        }

        self.open = next;
        (!escalate.is_empty() || !ask.is_empty()).then_some(Verdict {
            escalate,
            ask,
            compacted,
        })
    }
}

fn bullets(titles: &[String]) -> String {
    titles
        .iter()
        .map(|t| format!("- {t}"))
        .collect::<Vec<_>>()
        .join("\n")
}

impl Verdict {
    /// The escalation line injected at the start of the turn, or `""` when this turn only asks the
    /// user. Names the tasks, says what counts as resolving them, and rules out the one thing the
    /// looping session kept doing instead.
    pub(crate) fn render(&self) -> String {
        if self.escalate.is_empty() {
            return String::new();
        }
        let mut out = format!(
            "[tasks] These task(s) have been on your list for {ESCALATE_AFTER_TURNS} turns with \
             nothing about them changing:\n{}\n\nInvestigating what one of them means is not \
             progress on it. For EACH, do one of these in your next step and then move on: \
             finish it with a concrete tool call; mark it Done via update_tasks and say in one \
             line what you did; remove it from the list via update_tasks because it is moot; or \
             call ask_user to ask the user what it should mean. Do not open another round of \
             reading to decide.",
            bullets(&self.escalate)
        );
        if self.compacted {
            out.push_str(
                "\n\nThe conversation was compacted while they were open, so the detail behind \
                 them is no longer in your context. If you cannot reconstruct what one meant, ask \
                 the user or drop it — do not guess at it.",
            );
        }
        out
    }
}

/// What the user decided about a stalled task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    Keep,
    Done,
    Remove,
    /// A free-text answer, relayed to the model as-is.
    Other(String),
    /// Nobody answered: the task stays exactly as it is.
    NoAnswer,
}

impl Decision {
    pub(crate) fn parse(answer: &str) -> Self {
        let answer = answer.trim();
        if answer.is_empty() || answer == forge_types::NO_ANSWER {
            Decision::NoAnswer
        } else if answer.starts_with(KEEP) {
            Decision::Keep
        } else if answer.starts_with(DONE) {
            Decision::Done
        } else if answer.starts_with(REMOVE) {
            Decision::Remove
        } else {
            Decision::Other(answer.to_string())
        }
    }

    /// Apply the decision to the list; `true` when the list changed.
    pub(crate) fn apply(&self, tasks: &mut Vec<TodoItem>, title: &str) -> bool {
        match self {
            Decision::Done => {
                let mut changed = false;
                for task in tasks.iter_mut().filter(|t| t.title == title) {
                    task.status = TodoStatus::Done;
                    changed = true;
                }
                changed
            }
            Decision::Remove => {
                let before = tasks.len();
                tasks.retain(|t| t.title != title);
                tasks.len() != before
            }
            Decision::Keep | Decision::Other(_) | Decision::NoAnswer => false,
        }
    }

    /// What the model is told about the decision.
    pub(crate) fn note(&self, title: &str) -> String {
        let still = ESCALATE_AFTER_TURNS + ASK_AFTER_ESCALATION;
        let asked =
            format!("[tasks] \"{title}\" stood still for {still} turns, so Forge asked the user.");
        match self {
            Decision::Keep => format!(
                "{asked} They want you to keep working on it: continue it now with a concrete \
                 tool call."
            ),
            Decision::Done => format!("{asked} They said it is done, and it is now marked Done."),
            Decision::Remove => {
                format!("{asked} They removed it from the list. Do not re-add it unless they ask.")
            }
            Decision::Other(text) => {
                format!("{asked} Their answer:\n{text}\n\nAct on that for this task.")
            }
            Decision::NoAnswer => format!(
                "[tasks] \"{title}\" has not moved for {still} turns and nobody is present to \
                 decide about it, so it stays on the list. Finish it with a concrete tool call, or \
                 end your turn by saying plainly that it is still open and why."
            ),
        }
    }
}

/// The question put to the user about one stalled task.
pub(crate) fn question(title: &str, compacted: bool) -> (String, Vec<QChoice>) {
    let lost = if compacted {
        " The conversation was compacted while it was open, so the model may no longer know what \
         it meant."
    } else {
        ""
    };
    let text = format!(
        "The task \"{title}\" has not changed for {} turns.{lost} What should happen with it?",
        ESCALATE_AFTER_TURNS + ASK_AFTER_ESCALATION
    );
    let options = vec![
        QChoice {
            label: KEEP.to_string(),
            description: "Still wanted — the model is told to continue it.".to_string(),
        },
        QChoice {
            label: DONE.to_string(),
            description: "Mark it Done.".to_string(),
        },
        QChoice {
            label: REMOVE.to_string(),
            description: "Take it off the list.".to_string(),
        },
    ];
    (text, options)
}

fn status_word(status: TodoStatus) -> &'static str {
    if status == TodoStatus::Done {
        "done"
    } else if status == TodoStatus::InProgress {
        "in progress"
    } else {
        "pending"
    }
}

/// The task list a new message lands on, for the model, or `None` when nothing is open.
///
/// Without it, a message arriving mid-work read to the model like a fresh request: live
/// (2026-09-13), the first `update_tasks` after a user message replaced the entire list 51% of the
/// time and dropped at least one open task 69% of the time, against 3% and 24% mid-work — the user
/// steered, and the plan they were steering vanished.
pub(crate) fn carried_list_note(tasks: &[TodoItem]) -> Option<String> {
    if tasks.iter().all(|t| t.status == TodoStatus::Done) {
        return None;
    }
    let listed = tasks
        .iter()
        .take(LISTED_TASKS)
        .map(|t| format!("- [{}] {}", status_word(t.status), t.title))
        .collect::<Vec<_>>()
        .join("\n");
    let more = tasks.len().saturating_sub(LISTED_TASKS);
    let more = if more > 0 {
        format!("\n- (and {more} more)")
    } else {
        String::new()
    };
    Some(format!(
        "[task list] Your task list, carried into this message:\n{listed}{more}\n\nApply the \
         user's latest message to this list rather than starting a new one: keep the tasks it does \
         not affect, change or add the ones it calls for, and remove a task only when the message \
         makes it moot — and say which. Replace the whole list only if the user asked you to drop \
         the old plan."
    ))
}

/// Open tasks present `before` an update and missing `after` it.
pub(crate) fn removed_unfinished(before: &[TodoItem], after: &[TodoItem]) -> Vec<String> {
    before
        .iter()
        .filter(|t| t.status != TodoStatus::Done && !after.iter().any(|a| a.title == t.title))
        .map(|t| t.title.clone())
        .collect()
}

impl crate::Session {
    /// Turn-start pass over the task list: escalate stalled tasks to the model, put the ones it
    /// still did not move to the user, then show the model the list the new message lands on.
    /// Called after the user's prompt is persisted, so a question here cannot lose it.
    pub(crate) fn resolve_stale_tasks(
        &mut self,
        pack: &mut crate::context_pack::ContextPack,
    ) -> Result<(), crate::CoreError> {
        let mut stale = std::mem::take(&mut self.stale_tasks);
        let verdict = stale.turn(&self.tasks);
        self.stale_tasks = stale;
        if let Some(verdict) = verdict {
            let mut notes = vec![verdict.render()];
            let mut changed = false;
            for title in &verdict.ask {
                // Presence, not temper: a Bypass session with a human at the terminal can still
                // answer, and one with nobody attached must not have its list edited.
                let decision = if self.presenter.is_attended() {
                    let (text, options) = question(title, verdict.compacted);
                    Decision::parse(&self.presenter.ask(&text, &options, true))
                } else {
                    Decision::NoAnswer
                };
                changed |= decision.apply(&mut self.tasks, title);
                notes.push(decision.note(title));
            }
            if changed {
                self.persist_tasks();
                self.presenter
                    .emit(PresenterEvent::Tasks(self.tasks.clone()));
            }
            let text = notes
                .into_iter()
                .filter(|note| !note.is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            self.inject_context(
                pack,
                crate::context_pack::ContextSource::Tasks,
                "tasks that stopped moving",
                &text,
            )?;
        }
        if let Some(note) = carried_list_note(&self.tasks) {
            self.inject_context(
                pack,
                crate::context_pack::ContextSource::Tasks,
                "task list carried into this turn",
                &note,
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(title: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            title: title.to_string(),
            status,
            assignee: None,
        }
    }

    fn run(tracker: &mut Tracker, tasks: &[TodoItem], turns: u32) -> Option<Verdict> {
        let mut last = None;
        for _ in 0..turns {
            last = tracker.turn(tasks);
        }
        last
    }

    #[test]
    fn a_task_that_is_being_worked_on_is_never_touched() {
        let mut t = Tracker::default();
        // Two turns pending, then it moves to in progress, then two more. Never three unchanged.
        assert!(run(&mut t, &[task("a", TodoStatus::Pending)], 2).is_none());
        assert!(run(&mut t, &[task("a", TodoStatus::InProgress)], 2).is_none());
        assert!(t.turn(&[task("a", TodoStatus::Done)]).is_none());
    }

    #[test]
    fn a_task_that_stands_still_is_escalated_once_then_put_to_the_user() {
        let mut t = Tracker::default();
        let tasks = [task(
            "Strip live-reuse, keep perf wins",
            TodoStatus::InProgress,
        )];
        assert!(t.turn(&tasks).is_none());
        assert!(t.turn(&tasks).is_none());
        let v = t.turn(&tasks).expect("third unchanged turn escalates");
        assert_eq!(v.escalate, vec!["Strip live-reuse, keep perf wins"]);
        assert!(v.ask.is_empty());
        // The escalation is said once, not every turn after it.
        assert!(t.turn(&tasks).is_none());
        let v = t
            .turn(&tasks)
            .expect("two more turns and the user is asked");
        assert!(v.escalate.is_empty());
        assert_eq!(v.ask, vec!["Strip live-reuse, keep perf wins"]);
        assert!(
            v.render().is_empty(),
            "asking is the session's job, not a note"
        );
        // Asked once: the task starts over rather than being asked about again next turn.
        assert!(t.turn(&tasks).is_none());
        assert!(t.turn(&tasks).is_none());
    }

    #[test]
    fn a_turn_that_changed_the_workspace_does_not_age_its_tasks() {
        // The live false positive: shell-driven debugging with the status left at "in progress".
        let mut t = Tracker::default();
        let tasks = [task("reverse the login flow", TodoStatus::InProgress)];
        for _ in 0..10 {
            t.note_work(3);
            assert!(t.turn(&tasks).is_none());
        }
        // Once the work stops, it ages again from where it was.
        assert!(run(&mut t, &tasks, 2).is_some());
    }

    #[test]
    fn a_turn_with_no_workspace_change_still_ages_its_tasks() {
        let mut t = Tracker::default();
        let tasks = [task("a", TodoStatus::Pending)];
        t.note_work(0);
        assert!(run(&mut t, &tasks, 3).is_some());
    }

    #[test]
    fn answering_the_escalation_by_starting_the_task_clears_it() {
        let mut t = Tracker::default();
        let pending = [task("a", TodoStatus::Pending)];
        assert!(run(&mut t, &pending, 3).is_some());
        // The model reacts by picking it up; the counter restarts from that status.
        let started = [task("a", TodoStatus::InProgress)];
        assert!(t.turn(&started).is_none());
        assert!(t.turn(&started).is_none());
    }

    #[test]
    fn a_compaction_while_the_task_was_open_is_reported_with_it() {
        let mut t = Tracker::default();
        let tasks = [task("a", TodoStatus::InProgress)];
        assert!(t.turn(&tasks).is_none());
        t.note_compaction();
        assert!(t.turn(&tasks).is_none());
        let v = t.turn(&tasks).unwrap();
        assert!(v.compacted);
        assert!(v.render().contains("no longer in your context"));
    }

    #[test]
    fn a_compaction_before_the_task_existed_is_not_blamed_for_it() {
        let mut t = Tracker::default();
        t.note_compaction(); // nothing open yet
        let tasks = [task("a", TodoStatus::InProgress)];
        let v = run(&mut t, &tasks, 3).unwrap();
        assert!(!v.compacted);
        assert!(!v.render().contains("compacted"));
    }

    #[test]
    fn done_and_removed_tasks_are_forgotten() {
        let mut t = Tracker::default();
        let tasks = [
            task("a", TodoStatus::Pending),
            task("b", TodoStatus::Pending),
        ];
        assert!(run(&mut t, &tasks, 2).is_none());
        // `a` finishes and `b` is dropped by the model: nothing is left to escalate.
        assert!(t.turn(&[task("a", TodoStatus::Done)]).is_none());
        assert!(t.turn(&[task("a", TodoStatus::Done)]).is_none());
        assert!(t.turn(&[task("a", TodoStatus::Done)]).is_none());
        assert!(t.open.is_empty());
    }

    #[test]
    fn several_stalled_tasks_are_named_in_one_line() {
        let mut t = Tracker::default();
        let tasks = [
            task("a", TodoStatus::Pending),
            task("b", TodoStatus::Pending),
        ];
        let v = run(&mut t, &tasks, 3).unwrap();
        assert_eq!(v.escalate.len(), 2);
        let text = v.render();
        assert!(text.contains("- a") && text.contains("- b"));
        assert!(text.contains("ask_user"));
    }

    #[test]
    fn the_users_answer_decides_what_happens_to_the_task() {
        assert_eq!(Decision::parse(KEEP), Decision::Keep);
        assert_eq!(Decision::parse(DONE), Decision::Done);
        assert_eq!(Decision::parse(REMOVE), Decision::Remove);
        assert_eq!(Decision::parse(forge_types::NO_ANSWER), Decision::NoAnswer);
        assert_eq!(Decision::parse("  "), Decision::NoAnswer);
        assert_eq!(
            Decision::parse("split it into two"),
            Decision::Other("split it into two".into())
        );

        let list = || {
            vec![
                task("a", TodoStatus::Pending),
                task("b", TodoStatus::Pending),
            ]
        };
        let mut tasks = list();
        assert!(!Decision::Keep.apply(&mut tasks, "a"));
        assert_eq!(tasks, list());
        assert!(Decision::Done.apply(&mut tasks, "a"));
        assert_eq!(tasks[0].status, TodoStatus::Done);
        assert!(Decision::Remove.apply(&mut tasks, "b"));
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn nobody_present_leaves_the_task_on_the_list() {
        let mut tasks = vec![task("a", TodoStatus::InProgress)];
        assert!(!Decision::NoAnswer.apply(&mut tasks, "a"));
        assert_eq!(tasks.len(), 1);
        assert!(Decision::NoAnswer.note("a").contains("stays on the list"));
    }

    #[test]
    fn the_question_offers_the_three_ways_out_and_mentions_a_compaction() {
        let (text, options) = question("a", true);
        assert!(text.contains("\"a\"") && text.contains("compacted"));
        let labels: Vec<&str> = options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, vec![KEEP, DONE, REMOVE]);
        assert!(!question("a", false).0.contains("compacted"));
    }

    #[test]
    fn a_carried_list_names_open_work_and_says_to_apply_the_message_to_it() {
        assert!(carried_list_note(&[]).is_none());
        assert!(carried_list_note(&[task("a", TodoStatus::Done)]).is_none());
        let note = carried_list_note(&[
            task("a", TodoStatus::Done),
            task("b", TodoStatus::InProgress),
            task("c", TodoStatus::Pending),
        ])
        .unwrap();
        assert!(note.contains("- [done] a"));
        assert!(note.contains("- [in progress] b"));
        assert!(note.contains("- [pending] c"));
        assert!(note.contains("rather than starting a new one"));
    }

    #[test]
    fn a_long_carried_list_is_capped() {
        let tasks: Vec<TodoItem> = (0..15)
            .map(|i| task(&format!("t{i}"), TodoStatus::Pending))
            .collect();
        let note = carried_list_note(&tasks).unwrap();
        assert!(note.contains("t11") && !note.contains("t12"));
        assert!(note.contains("(and 3 more)"));
    }

    #[test]
    fn only_open_tasks_that_vanished_count_as_removed() {
        let before = [
            task("kept", TodoStatus::Pending),
            task("finished", TodoStatus::Done),
            task("lost", TodoStatus::InProgress),
        ];
        let after = [
            task("kept", TodoStatus::InProgress),
            task("new", TodoStatus::Pending),
        ];
        assert_eq!(removed_unfinished(&before, &after), vec!["lost"]);
    }
}
