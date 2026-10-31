//! Per-project notes: one markdown scratchpad and a checklist the person keeps beside the work
//! (docs/features/project-notes.md). Rows are keyed by the project root, so a session in a
//! throwaway worktree and one in the checkout share the same notes.

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::OptionalExtension;

use crate::{Result, Store, StoreError};

/// Longest scratchpad kept; a paste of a whole log should not become a database row.
pub const MAX_NOTE_BYTES: usize = 200_000;
/// Longest single to-do.
pub const MAX_TODO_CHARS: usize = 2_000;
/// Most to-dos one project holds.
pub const MAX_TODOS: usize = 500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectTodo {
    pub id: String,
    pub project: String,
    pub text: String,
    pub done: bool,
    pub position: i64,
    /// The session started from this item, if any.
    pub session_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectNotes {
    pub project: String,
    pub body: String,
    pub updated_at: i64,
    pub todos: Vec<ProjectTodo>,
}

/// The key notes are stored under: the checkout a worktree session belongs to, without a trailing
/// slash. A session's `.forge/worktrees/<name>` directory is not a project of its own.
pub fn project_key(path: &str) -> String {
    let trimmed = path.trim();
    let root = match trimmed.find("/.forge/worktrees/") {
        Some(at) => &trimmed[..at],
        None => trimmed,
    };
    let root = root.trim_end_matches('/');
    if root.is_empty() && trimmed.starts_with('/') {
        "/".to_string()
    } else {
        root.to_string()
    }
}

fn new_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("td_{nanos:x}{n:x}")
}

fn todo_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectTodo> {
    Ok(ProjectTodo {
        id: r.get(0)?,
        project: r.get(1)?,
        text: r.get(2)?,
        done: r.get::<_, i64>(3)? != 0,
        position: r.get(4)?,
        session_id: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

const TODO_COLUMNS: &str = "id, project, text, done, position, session_id, created_at, updated_at";

fn invalid(what: &str) -> StoreError {
    StoreError::InvalidValue(what.to_string())
}

fn clean_todo(text: &str) -> Result<String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(invalid("a to-do needs some text"));
    }
    if text.chars().count() > MAX_TODO_CHARS {
        return Err(invalid("a to-do is limited to 2000 characters"));
    }
    Ok(text.to_string())
}

impl Store {
    /// The scratchpad and checklist for `project` (open items first, in the order they were
    /// added; finished items after them).
    pub fn project_notes(&self, project: &str) -> Result<ProjectNotes> {
        let project = project_key(project);
        let conn = self.lock()?;
        let note: Option<(String, i64)> = conn
            .query_row(
                "SELECT body, updated_at FROM project_note WHERE project = ?1",
                [&project],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {TODO_COLUMNS} FROM project_todo WHERE project = ?1 \
             ORDER BY done ASC, position ASC, created_at ASC, id ASC"
        ))?;
        let todos = stmt
            .query_map([&project], todo_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let (body, updated_at) = note.unwrap_or_default();
        Ok(ProjectNotes {
            project,
            body,
            updated_at,
            todos,
        })
    }

    /// Replace the project's scratchpad. Returns the stored timestamp.
    pub fn set_project_scratchpad(&self, project: &str, body: &str, now: i64) -> Result<i64> {
        if body.len() > MAX_NOTE_BYTES {
            return Err(invalid("the scratchpad is limited to 200 KB"));
        }
        let project = project_key(project);
        if project.is_empty() {
            return Err(invalid("a project is required"));
        }
        self.lock()?.execute(
            "INSERT INTO project_note (project, body, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(project) DO UPDATE SET body = excluded.body, updated_at = excluded.updated_at",
            rusqlite::params![project, body, now],
        )?;
        Ok(now)
    }

    /// Append an open to-do and return it.
    pub fn add_project_todo(&self, project: &str, text: &str, now: i64) -> Result<ProjectTodo> {
        let project = project_key(project);
        if project.is_empty() {
            return Err(invalid("a project is required"));
        }
        let text = clean_todo(text)?;
        let conn = self.lock()?;
        let (count, last): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(position), 0) FROM project_todo WHERE project = ?1",
            [&project],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if count as usize >= MAX_TODOS {
            return Err(invalid("this project already holds 500 to-dos"));
        }
        let todo = ProjectTodo {
            id: new_id(),
            project,
            text,
            done: false,
            position: last + 1,
            session_id: None,
            created_at: now,
            updated_at: now,
        };
        conn.execute(
            "INSERT INTO project_todo (id, project, text, done, position, session_id, created_at, \
             updated_at) VALUES (?1, ?2, ?3, 0, ?4, NULL, ?5, ?5)",
            rusqlite::params![todo.id, todo.project, todo.text, todo.position, now],
        )?;
        Ok(todo)
    }

    /// Edit an item's text and/or check it off. `None` leaves a field alone; returns the item, or
    /// `None` when no such item exists.
    pub fn update_project_todo(
        &self,
        id: &str,
        text: Option<&str>,
        done: Option<bool>,
        now: i64,
    ) -> Result<Option<ProjectTodo>> {
        let text = text.map(clean_todo).transpose()?;
        let conn = self.lock()?;
        let changed = conn.execute(
            "UPDATE project_todo SET text = COALESCE(?2, text), done = COALESCE(?3, done), \
             updated_at = ?4 WHERE id = ?1",
            rusqlite::params![id, text, done.map(i64::from), now],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        Ok(conn
            .query_row(
                &format!("SELECT {TODO_COLUMNS} FROM project_todo WHERE id = ?1"),
                [id],
                todo_from_row,
            )
            .optional()?)
    }

    /// Remember which session an item was turned into.
    pub fn link_project_todo_session(&self, id: &str, session_id: &str, now: i64) -> Result<bool> {
        Ok(self.lock()?.execute(
            "UPDATE project_todo SET session_id = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![id, session_id, now],
        )? > 0)
    }

    pub fn delete_project_todo(&self, id: &str) -> Result<bool> {
        Ok(self
            .lock()?
            .execute("DELETE FROM project_todo WHERE id = ?1", [id])?
            > 0)
    }

    /// Drop every finished item of the project; returns how many went.
    pub fn clear_done_project_todos(&self, project: &str) -> Result<usize> {
        Ok(self.lock()?.execute(
            "DELETE FROM project_todo WHERE project = ?1 AND done = 1",
            [project_key(project)],
        )?)
    }

    /// The project a to-do belongs to, for callers that only hold its id.
    pub fn project_todo(&self, id: &str) -> Result<Option<ProjectTodo>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("SELECT {TODO_COLUMNS} FROM project_todo WHERE id = ?1"),
                [id],
                todo_from_row,
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::project_key;
    use crate::Store;

    #[test]
    fn a_worktree_shares_its_checkouts_notes() {
        assert_eq!(project_key("/w/app/"), "/w/app");
        assert_eq!(project_key("/w/app/.forge/worktrees/fix-1"), "/w/app");
        assert_eq!(project_key("/"), "/");
    }

    #[test]
    fn the_scratchpad_round_trips_and_overwrites() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.project_notes("/w/app").unwrap().body, "");
        store
            .set_project_scratchpad("/w/app", "# plan", 10)
            .unwrap();
        store
            .set_project_scratchpad("/w/app/.forge/worktrees/x", "# plan v2", 20)
            .unwrap();
        let notes = store.project_notes("/w/app").unwrap();
        assert_eq!((notes.body.as_str(), notes.updated_at), ("# plan v2", 20));
        assert!(store.project_notes("/w/other").unwrap().body.is_empty());
    }

    #[test]
    fn todos_keep_order_check_off_and_sort_finished_last() {
        let store = Store::open_in_memory().unwrap();
        let a = store.add_project_todo("/w/app", " first ", 1).unwrap();
        let b = store.add_project_todo("/w/app", "second", 2).unwrap();
        store.add_project_todo("/w/app", "third", 3).unwrap();
        assert_eq!(a.text, "first");
        store
            .update_project_todo(&a.id, None, Some(true), 4)
            .unwrap()
            .unwrap();
        let texts: Vec<_> = store
            .project_notes("/w/app")
            .unwrap()
            .todos
            .into_iter()
            .map(|t| (t.text, t.done))
            .collect();
        assert_eq!(
            texts,
            [
                ("second".to_string(), false),
                ("third".to_string(), false),
                ("first".to_string(), true)
            ]
        );
        let edited = store
            .update_project_todo(&b.id, Some("second, reworded"), None, 5)
            .unwrap()
            .unwrap();
        assert_eq!(edited.text, "second, reworded");
        assert!(!edited.done);
        assert_eq!(store.clear_done_project_todos("/w/app").unwrap(), 1);
        assert_eq!(store.project_notes("/w/app").unwrap().todos.len(), 2);
    }

    #[test]
    fn a_todo_links_to_its_session_and_can_be_deleted() {
        let store = Store::open_in_memory().unwrap();
        let todo = store
            .add_project_todo("/w/app", "fix flaky test", 1)
            .unwrap();
        assert!(store
            .link_project_todo_session(&todo.id, "sess-1", 2)
            .unwrap());
        let found = store.project_todo(&todo.id).unwrap().unwrap();
        assert_eq!(found.session_id.as_deref(), Some("sess-1"));
        assert!(store.delete_project_todo(&todo.id).unwrap());
        assert!(!store.delete_project_todo(&todo.id).unwrap());
        assert!(store.project_todo(&todo.id).unwrap().is_none());
        assert!(store
            .update_project_todo("nope", None, Some(true), 3)
            .unwrap()
            .is_none());
    }

    #[test]
    fn empty_and_oversized_input_is_refused() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.add_project_todo("/w/app", "   ", 1).is_err());
        assert!(store.add_project_todo("", "x", 1).is_err());
        assert!(store
            .add_project_todo("/w/app", &"x".repeat(2001), 1)
            .is_err());
        assert!(store
            .set_project_scratchpad("/w/app", &"x".repeat(200_001), 1)
            .is_err());
    }
}
