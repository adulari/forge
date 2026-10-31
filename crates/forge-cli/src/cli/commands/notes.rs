//! `forge notes`: the project scratchpad and to-do list from the shell. The agent reads the same
//! rows through this command, and the desktop, web and mobile Notes surface edits them.

use anyhow::{bail, Context, Result};
use forge_store::{ProjectNotes, ScratchpadSave, Store};

use crate::cli::args::NotesCmd;
use crate::cli::commands::replay::open_store;

/// The plain-text form of a project's notes: what `forge notes list` prints and what the agent
/// reads. To-dos are numbered so `forge notes done 2` is unambiguous.
pub(crate) fn render_notes(notes: &ProjectNotes) -> String {
    let mut out = format!("Notes for {}\n", notes.project);
    out.push_str("\n## Scratchpad\n");
    if notes.body.trim().is_empty() {
        out.push_str("(empty)\n");
    } else {
        out.push_str(notes.body.trim_end());
        out.push('\n');
    }
    out.push_str("\n## To-dos\n");
    if notes.todos.is_empty() {
        out.push_str("(none)\n");
    }
    for (index, todo) in notes.todos.iter().enumerate() {
        let mark = if todo.done { "x" } else { " " };
        let linked = todo
            .session_id
            .as_deref()
            .map(|id| format!("  (thread {})", id.chars().take(8).collect::<String>()))
            .unwrap_or_default();
        out.push_str(&format!("{}. [{mark}] {}{linked}\n", index + 1, todo.text));
    }
    out
}

fn project_of(project: Option<String>) -> Result<String> {
    match project {
        Some(project) => Ok(project),
        None => Ok(std::env::current_dir()
            .context("reading the current directory")?
            .to_string_lossy()
            .into_owned()),
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Append a line to the scratchpad without clobbering a concurrent edit: the write names the
/// revision it read, and when another writer got in first the append is redone on top of theirs.
fn append_to_pad(store: &Store, project: &str, line: &str) -> Result<()> {
    for _ in 0..5 {
        let notes = store.project_notes(project)?;
        let mut body = notes.body.trim_end().to_string();
        if !body.is_empty() {
            body.push('\n');
        }
        body.push_str(line);
        if let ScratchpadSave::Saved { .. } =
            store.set_project_scratchpad_if(project, &body, Some(notes.updated_at), now_ms())?
        {
            return Ok(());
        }
    }
    bail!("the scratchpad keeps changing; try again")
}

/// Resolve `item` against the project's to-dos: a 1-based number, or an id prefix.
fn resolve(store: &Store, project: &str, item: &str) -> Result<String> {
    let notes = store.project_notes(project)?;
    if let Ok(number) = item.parse::<usize>() {
        return match number.checked_sub(1).and_then(|i| notes.todos.get(i)) {
            Some(todo) => Ok(todo.id.clone()),
            None => bail!("no to-do number {number} (there are {})", notes.todos.len()),
        };
    }
    let matches: Vec<_> = notes
        .todos
        .iter()
        .filter(|todo| todo.id.starts_with(item))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one.id.clone()),
        [] => bail!("no to-do matches `{item}`"),
        _ => bail!("`{item}` matches more than one to-do; use its number"),
    }
}

pub(crate) fn notes_cmd(cmd: Option<NotesCmd>) -> Result<()> {
    let store = open_store()?;
    let cmd = cmd.unwrap_or(NotesCmd::List { project: None });
    match cmd {
        NotesCmd::List { project } => {
            let notes = store.project_notes(&project_of(project)?)?;
            print!("{}", render_notes(&notes));
        }
        NotesCmd::Add { text, project } => {
            let todo = store.add_project_todo(&project_of(project)?, &text.join(" "), now_ms())?;
            println!("added: {}", todo.text);
        }
        NotesCmd::Done { item, project } => {
            let project = project_of(project)?;
            let id = resolve(&store, &project, &item)?;
            let todo = store.update_project_todo(&id, None, Some(true), now_ms())?;
            println!("done: {}", todo.map_or_else(String::new, |t| t.text));
        }
        NotesCmd::Reopen { item, project } => {
            let project = project_of(project)?;
            let id = resolve(&store, &project, &item)?;
            let todo = store.update_project_todo(&id, None, Some(false), now_ms())?;
            println!("reopened: {}", todo.map_or_else(String::new, |t| t.text));
        }
        NotesCmd::Rm { item, project } => {
            let project = project_of(project)?;
            let id = resolve(&store, &project, &item)?;
            store.delete_project_todo(&id)?;
            println!("removed");
        }
        NotesCmd::Pad { text, project } => {
            let project = project_of(project)?;
            if text.is_empty() {
                println!("{}", store.project_notes(&project)?.body.trim_end());
            } else {
                append_to_pad(&store, &project, &text.join(" "))?;
                println!("appended");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_listing_numbers_todos_and_marks_finished_ones() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_project_scratchpad("/w/app", "remember the **flag**", 1)
            .unwrap();
        let a = store.add_project_todo("/w/app", "open one", 1).unwrap();
        let b = store.add_project_todo("/w/app", "finished", 2).unwrap();
        store
            .update_project_todo(&b.id, None, Some(true), 3)
            .unwrap();
        store
            .link_project_todo_session(&a.id, "abcdef123456", 4)
            .unwrap();
        let text = render_notes(&store.project_notes("/w/app").unwrap());
        assert!(text.contains("remember the **flag**"));
        assert!(text.contains("1. [ ] open one  (thread abcdef12)"));
        assert!(text.contains("2. [x] finished"));
    }

    #[test]
    fn appending_keeps_what_is_already_in_the_pad() {
        let store = Store::open_in_memory().unwrap();
        append_to_pad(&store, "/w/app", "first").unwrap();
        append_to_pad(&store, "/w/app", "second").unwrap();
        assert_eq!(store.project_notes("/w/app").unwrap().body, "first\nsecond");
    }

    #[test]
    fn items_resolve_by_number_or_id_prefix() {
        let store = Store::open_in_memory().unwrap();
        let a = store.add_project_todo("/w/app", "one", 1).unwrap();
        store.add_project_todo("/w/app", "two", 2).unwrap();
        assert_eq!(resolve(&store, "/w/app", "1").unwrap(), a.id);
        assert_eq!(
            resolve(&store, "/w/app", &a.id[..a.id.len() - 1]).unwrap(),
            a.id
        );
        assert!(resolve(&store, "/w/app", "3").is_err());
        assert!(resolve(&store, "/w/app", "0").is_err());
        assert!(resolve(&store, "/w/app", "zz").is_err());
    }
}
