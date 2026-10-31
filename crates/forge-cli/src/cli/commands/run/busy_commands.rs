//! What a `/command` typed while a turn is running does. One classification, shared by every
//! input surface (local keyboard, remote prompt, palette, daemon driver) and by `dispatch_command`
//! itself, so the surfaces cannot drift apart.
//!
//! The running turn holds the session `Mutex` for its whole length, so a command that does
//! `session.lock().await` would freeze the render loop (the #45 invariant) until the turn ends.
//! Such commands are not refused: they wait in the same queue as typed prompts and run through
//! normal dispatch at the turn boundary, in order. They are never added to the steer inbox, which
//! feeds text to the running model turn.

use forge_tui::{App, CommandAction, GoalControl};

/// How a command behaves while a turn is active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BusyPolicy {
    /// Runs right now beside the turn: never awaits the session lock (`/btw` goes through the
    /// session's detached asker instead).
    Immediate,
    /// Needs the session lock or changes turn state: queued, runs when the turn finishes.
    Queue,
    /// Running it later makes no sense or is unsafe: refused with a note.
    Refuse,
}

/// The single source of truth. Exhaustive on purpose: a new `CommandAction` must be classified
/// here before it compiles.
pub(crate) fn busy_policy(action: &CommandAction) -> BusyPolicy {
    use BusyPolicy::{Immediate, Queue, Refuse};
    match action {
        CommandAction::Help
        | CommandAction::Status
        | CommandAction::Keys
        | CommandAction::ListSessions
        | CommandAction::Board
        | CommandAction::Config
        | CommandAction::Bug(_)
        | CommandAction::ClearScreen
        | CommandAction::Btw(_)
        | CommandAction::Remote { .. }
        | CommandAction::Anywhere
        | CommandAction::GoalControl(
            GoalControl::Pause | GoalControl::Clear | GoalControl::Status | GoalControl::Edit(_),
        )
        | CommandAction::Thinking
        | CommandAction::ToolOutput
        | CommandAction::Image(_)
        | CommandAction::Statusline(_)
        | CommandAction::Voice
        | CommandAction::Stash(_)
        | CommandAction::Skills
        | CommandAction::Theme(_)
        // Neither takes the session lock: git and the checks run on their own thread.
        | CommandAction::Diff(_)
        | CommandAction::Doctor
        // Writes the title through the session's detached store handle, never the session lock.
        | CommandAction::Rename(_)
        | CommandAction::Star
        | CommandAction::Delete
        | CommandAction::Done
        | CommandAction::Archived(_) => Immediate,
        CommandAction::Dispatch(_)
        | CommandAction::PinModel(_)
        | CommandAction::ListModels
        | CommandAction::Mcp(_)
        | CommandAction::New
        | CommandAction::Mode
        | CommandAction::Assay { .. }
        | CommandAction::Checkpoint(_)
        | CommandAction::Compact(_)
        | CommandAction::Commit(_)
        | CommandAction::Uncompact
        | CommandAction::Reground
        | CommandAction::Refine(_)
        | CommandAction::Lattice(_)
        | CommandAction::Goal { .. }
        | CommandAction::GoalControl(GoalControl::Resume)
        | CommandAction::Pr(_)
        | CommandAction::Loop { .. }
        | CommandAction::Replay(_, _)
        | CommandAction::Export(_)
        | CommandAction::Usage
        | CommandAction::Stats(_)
        | CommandAction::Mesh(_)
        | CommandAction::Init
        | CommandAction::SetEffort(_)
        | CommandAction::SetExactEffort(_)
        | CommandAction::Subagents(_)
        | CommandAction::Delegation(_)
        | CommandAction::Crew(_)
        | CommandAction::Agents
        | CommandAction::Plan(_)
        | CommandAction::Execute
        | CommandAction::Copy { .. }
        | CommandAction::Remember(_)
        | CommandAction::Memories
        | CommandAction::SelfMcp(_)
        | CommandAction::Workflow(_)
        | CommandAction::Heartbeat(_)
        | CommandAction::Duel(_)
        | CommandAction::Context
        | CommandAction::Permissions(_)
        | CommandAction::McpReconnect(_)
        | CommandAction::AddDir(_)
        // A file command/skill; `route_busy_line` refines this once the catalog says which.
        | CommandAction::Unknown(_) => Queue,
        // Resume/Undo/Checkpoints pick a different point of the conversation to return to, which
        // is meaningless once the running turn has moved on; /claude and /quit end the process.
        CommandAction::Resume(_)
        | CommandAction::Fork(_)
        | CommandAction::Undo
        | CommandAction::ListCheckpoints
        | CommandAction::Claude
        | CommandAction::Quit => Refuse,
    }
}

/// What an input surface does with a command line typed while a turn runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BusyRoute {
    RunNow,
    Queue,
    Refuse,
    /// Would be queued, but a `/loop` or `/goal` owns the queue (its steering drains it).
    RefuseAutonomous,
}

pub(crate) fn is_command_line(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('/') && !t.starts_with("//")
}

pub(crate) fn route_busy_line(
    line: &str,
    catalog: &forge_skills::Catalog,
    autonomous: bool,
) -> BusyRoute {
    let action = forge_tui::parse_command(line);
    let policy = match &action {
        CommandAction::Unknown(_) => match catalog.resolve(line) {
            forge_skills::Resolved::Command { .. } | forge_skills::Resolved::Skill { .. } => {
                BusyPolicy::Queue
            }
            // An unknown name or missing args only prints a note.
            _ => BusyPolicy::Immediate,
        },
        other => busy_policy(other),
    };
    match policy {
        BusyPolicy::Immediate => BusyRoute::RunNow,
        BusyPolicy::Queue if autonomous => BusyRoute::RefuseAutonomous,
        BusyPolicy::Queue => BusyRoute::Queue,
        BusyPolicy::Refuse => BusyRoute::Refuse,
    }
}

pub(crate) const REFUSE_AUTONOMOUS_NOTE: &str =
    "⏳ commands can't be queued while /loop or /goal is running — Esc to stop it first";

/// Queue a command behind the prompts already waiting. Deliberately does not touch the steer
/// inbox: a slash command must never be sent to the model as text.
pub(crate) fn queue_command(queue: &mut Vec<String>, app: &mut App, line: &str) {
    let line = line.trim().to_string();
    let name = line.split_whitespace().next().unwrap_or("/").to_string();
    queue.push(line);
    app.set_queued(queue);
    if app.queue_visible {
        return;
    }
    app.note(&format!(
        "⏳ queued {name} — runs when this turn finishes ({} pending)",
        queue.len()
    ));
}

/// The next thing a turn boundary should run.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Queued {
    Command(String),
    Prompt(String),
}

/// Pop the head of the queue, commands included, so commands and prompts keep their typed order.
pub(crate) fn dequeue_next(queue: &mut Vec<String>, app: &mut App) -> Option<Queued> {
    let head = queue.first()?;
    if is_command_line(head) {
        let line = queue.remove(0);
        app.set_queued(queue);
        return Some(Queued::Command(line));
    }
    super::dequeue_prompt(queue, app).map(Queued::Prompt)
}

pub(crate) fn head_is_command(queue: &[String]) -> bool {
    queue.first().is_some_and(|h| is_command_line(h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use BusyPolicy::{Immediate, Queue, Refuse};

    fn policy(line: &str) -> BusyPolicy {
        busy_policy(&forge_tui::parse_command(line))
    }

    #[test]
    fn every_command_family_is_classified() {
        let immediate = [
            "/help",
            "/keys",
            "/sessions",
            "/board",
            "/config",
            "/cls",
            "/diff",
            "/doctor",
            "/btw why is the sky blue",
            "/side hi",
            "/remote",
            "/anywhere",
            "/thinking",
            "/output",
            "/image a.png",
            "/statusline",
            "/voice",
            "/rename x",
            "/goal pause",
            "/goal clear",
            "/goal status",
            "/goal edit sharper wording",
        ];
        let queued = [
            "/goal resume",
            "/dispatch build it",
            "/model",
            "/model groq::x",
            "/models",
            "/mcp",
            "/mcp reconnect",
            "/new",
            "/clear",
            "/context",
            "/permissions",
            "/add-dir ../lib",
            "/mode",
            "/assay",
            "/checkpoint x",
            "/compact",
            "/commit",
            "/uncompact",
            "/reground",
            "/refine",
            "/lattice foo",
            "/goal ship it",
            "/pr",
            "/loop do it",
            "/replay abc",
            "/export",
            "/usage",
            "/mesh",
            "/init",
            "/effort high",
            "/effort exact high",
            "/subagents free",
            "/plan do x",
            "/execute",
            "/copy",
            "/remember x",
            "/memories",
            "/self-mcp",
            "/workflow list",
            "/heartbeat",
            "/duel x",
            "/some-file-command",
        ];
        let refused = ["/resume abc", "/undo", "/checkpoints", "/claude", "/quit"];
        for l in immediate {
            assert_eq!(policy(l), Immediate, "{l}");
        }
        for l in queued {
            assert_eq!(policy(l), Queue, "{l}");
        }
        for l in refused {
            assert_eq!(policy(l), Refuse, "{l}");
        }
    }

    #[test]
    fn btw_runs_now_while_busy() {
        let catalog = forge_skills::Catalog::default();
        assert_eq!(
            route_busy_line("/btw what is a mutex", &catalog, false),
            BusyRoute::RunNow
        );
        assert_eq!(
            route_busy_line("/btw what is a mutex", &catalog, true),
            BusyRoute::RunNow
        );
    }

    #[test]
    fn queue_class_is_queued_unless_an_autonomous_run_owns_the_queue() {
        let catalog = forge_skills::Catalog::default();
        assert_eq!(
            route_busy_line("/compact", &catalog, false),
            BusyRoute::Queue
        );
        assert_eq!(
            route_busy_line("/compact", &catalog, true),
            BusyRoute::RefuseAutonomous
        );
        assert_eq!(route_busy_line("/quit", &catalog, false), BusyRoute::Refuse);
    }

    #[test]
    fn unknown_names_only_print_a_note_so_they_run_now() {
        let catalog = forge_skills::Catalog::default();
        assert_eq!(
            route_busy_line("/definitely-not-a-command", &catalog, false),
            BusyRoute::RunNow
        );
    }

    #[test]
    fn a_queued_command_is_queued_not_steered_and_drains_in_order_with_prompts() {
        let steer = forge_core::steer::SteerInbox::default();
        let mut app = App::default();
        let mut queue: Vec<String> = Vec::new();
        queue.push("first prompt".into());
        steer.push("first prompt");
        queue_command(&mut queue, &mut app, "  /compact  ");
        queue.push("last prompt".into());
        steer.push("last prompt");

        assert_eq!(queue, ["first prompt", "/compact", "last prompt"]);
        let steered = steer.drain();
        assert_eq!(steered, ["first prompt", "last prompt"]);

        assert_eq!(
            dequeue_next(&mut queue, &mut app),
            Some(Queued::Prompt("first prompt".into()))
        );
        assert!(head_is_command(&queue));
        assert_eq!(
            dequeue_next(&mut queue, &mut app),
            Some(Queued::Command("/compact".into()))
        );
        assert_eq!(
            dequeue_next(&mut queue, &mut app),
            Some(Queued::Prompt("last prompt".into()))
        );
        assert_eq!(dequeue_next(&mut queue, &mut app), None);
    }

    #[test]
    fn steering_helpers_never_hand_a_command_to_the_model() {
        let mut app = App::default();
        let mut queue = vec!["/compact".to_string(), "fix the bug".to_string()];
        assert_eq!(
            super::super::dequeue_prompt(&mut queue, &mut app),
            Some("fix the bug".to_string())
        );
        assert_eq!(queue, ["/compact"]);
        assert_eq!(super::super::dequeue_prompt(&mut queue, &mut app), None);
    }
}
