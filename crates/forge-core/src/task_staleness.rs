//! A task the model can no longer resolve must not re-drive the session forever.
//!
//! Live failure (2026-09-09, session `07ca114e`): an auto-compaction left one task — "Strip
//! live-reuse, keep perf wins" — `InProgress`, with the reasoning that produced it no longer in
//! context. From then on every turn ended with the completion gate seeing an unfinished task, and
//! every continue-nudge spent its steps re-reading the same files to work out what the task meant.
//! Those reads count as progress in [`crate::nudge_policy`], so the nudge budget never ran out;
//! the session looped until a human interrupted it and made the call by hand.
//!
//! The missing signal is structural and cheap: a task whose title and status have not moved for
//! several turns is not being worked on, whatever the model says about it. This module counts that
//! and hands the session two escalations — first force a decision naming the task, then, if the
//! next turn changes nothing either, drop it from the list so the gate stops re-driving it.
//!
//! Dropping is deliberately not "mark it Done": Done is a completion claim the harness has no
//! evidence for, and it would send the turn into the verification gate instead. A dropped task
//! leaves the list with a warning to the user, who is the one who can decide what it meant.

use forge_types::{TodoItem, TodoStatus};
use std::collections::HashMap;

/// Turns an unfinished task may sit unchanged before the harness forces a decision about it.
///
/// Three, not one: a task legitimately spans several turns of real work, and the status does not
/// have to change while it does. What is not legitimate is three consecutive turns in which
/// nothing about it moved at all.
pub(crate) const ESCALATE_AFTER_TURNS: u32 = 3;

/// Further unchanged turns after the escalation before the harness removes the task itself.
pub(crate) const DROP_AFTER_ESCALATION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    status: TodoStatus,
    /// Consecutive turns this exact (title, status) pair has survived.
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
    /// Tasks the harness is removing from the list right now.
    pub dropped: Vec<String>,
    /// Any of them were open across a compaction.
    pub compacted: bool,
}

/// Counts how long each unfinished task has stood still. One instance per session.
#[derive(Debug, Clone, Default)]
pub(crate) struct Tracker {
    open: HashMap<String, Entry>,
}

impl Tracker {
    /// Record that the transcript was compacted: every task open right now loses the context that
    /// explains it, which is exactly the case that produced the live loop.
    pub(crate) fn note_compaction(&mut self) {
        for entry in self.open.values_mut() {
            entry.compacted = true;
        }
    }

    /// Advance one turn over the session's current task list, returning what to do about anything
    /// that has stopped moving. Tasks that are Done, renamed, re-statused, or gone from the list
    /// reset — only a genuinely untouched one accumulates turns.
    pub(crate) fn turn(&mut self, tasks: &[TodoItem]) -> Option<Verdict> {
        let mut next: HashMap<String, Entry> = HashMap::new();
        let mut escalate = Vec::new();
        let mut dropped = Vec::new();
        let mut compacted = false;

        for task in tasks.iter().filter(|t| t.status != TodoStatus::Done) {
            let mut entry = match self.open.remove(&task.title) {
                Some(prev) if prev.status == task.status => Entry {
                    turns: prev.turns.saturating_add(1),
                    ..prev
                },
                _ => Entry {
                    status: task.status,
                    turns: 1,
                    compacted: false,
                    escalated: false,
                },
            };
            if entry.escalated && entry.turns >= ESCALATE_AFTER_TURNS + DROP_AFTER_ESCALATION {
                compacted |= entry.compacted;
                dropped.push(task.title.clone());
                continue; // gone from the list, so it leaves the tracker too
            }
            if !entry.escalated && entry.turns >= ESCALATE_AFTER_TURNS {
                entry.escalated = true;
                compacted |= entry.compacted;
                escalate.push(task.title.clone());
            }
            next.insert(task.title.clone(), entry);
        }

        self.open = next;
        (!escalate.is_empty() || !dropped.is_empty()).then_some(Verdict {
            escalate,
            dropped,
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
    /// The system line injected at the start of the turn. Names the tasks, says what counts as
    /// resolving them, and rules out the one thing the looping session kept doing instead.
    pub(crate) fn render(&self) -> String {
        let mut out = String::new();
        if !self.escalate.is_empty() {
            out.push_str(&format!(
                "[tasks] These task(s) have been on your list for {ESCALATE_AFTER_TURNS} turns with \
                 nothing about them changing:\n{}\n\nInvestigating what one of them means is not \
                 progress on it. For EACH, do one of these in your next step and then move on: \
                 finish it with a concrete tool call; mark it Done via update_tasks and say in one \
                 line what you did; remove it from the list via update_tasks because it is moot; or \
                 call ask_user to ask the user what it should mean. Do not open another round of \
                 reading to decide.",
                bullets(&self.escalate)
            ));
        }
        if !self.dropped.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&format!(
                "[tasks] Forge removed these task(s) from your list — they stood still for \
                 {} turns, including after you were asked to resolve or drop \
                 them:\n{}\n\nThey are no longer tracked and will not hold the turn open. Do not \
                 re-add them unless the user asks for them. The rest of your list still stands.",
                ESCALATE_AFTER_TURNS + DROP_AFTER_ESCALATION,
                bullets(&self.dropped)
            ));
        }
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

/// Told to the user, not the model: the harness edited their task list, so it has to say so.
pub(crate) fn dropped_warning(dropped: &[String]) -> String {
    format!(
        "dropped {} stalled task(s) from the list — unchanged for {} turns: {}. Re-add one if it \
         still matters.",
        dropped.len(),
        ESCALATE_AFTER_TURNS + DROP_AFTER_ESCALATION,
        dropped.join(", ")
    )
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
    fn a_task_that_stands_still_is_escalated_once_then_dropped() {
        let mut t = Tracker::default();
        let tasks = [task(
            "Strip live-reuse, keep perf wins",
            TodoStatus::InProgress,
        )];
        assert!(t.turn(&tasks).is_none());
        assert!(t.turn(&tasks).is_none());
        let v = t.turn(&tasks).expect("third unchanged turn escalates");
        assert_eq!(v.escalate, vec!["Strip live-reuse, keep perf wins"]);
        assert!(v.dropped.is_empty());
        // The escalation is said once, not every turn after it.
        assert!(t.turn(&tasks).is_none());
        let v = t.turn(&tasks).expect("two more turns and it is dropped");
        assert!(v.escalate.is_empty());
        assert_eq!(v.dropped, vec!["Strip live-reuse, keep perf wins"]);
        // Dropped means gone: the session removes it, and a list that still carries it starts over
        // rather than being dropped again on the next turn.
        assert!(t.turn(&tasks).is_none());
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
}
