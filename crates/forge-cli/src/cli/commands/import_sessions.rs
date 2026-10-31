//! Conversations other agents left on this machine, listed and imported in bulk.
//!
//! One engine behind `forge import scan` / `forge import sessions` / `forge import codex-session`,
//! the daemon's `GET /api/import/scan` and `POST /api/import` (desktop, web and mobile), and the
//! TUI's `/import`. Claude Code transcripts go through [`super::import_claude_session`]; Codex
//! rollouts through [`super::codex_rollout`]. Source files are only ever read.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use forge_store::{ImportedRow, ImportedSessionMeta, Store};
use forge_types::Role;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::codex_rollout;
use super::import_claude_session::{clean_title, import_transcript, ClaudeSessionArgs};

/// How much of a Claude transcript's start is read to list it.
const HEAD_BYTES: u64 = 256 * 1024;
const CODEX_PREFIX: &str = "codex-cli::";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// One conversation found on disk.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Candidate {
    pub(crate) agent: Agent,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) cwd: Option<String>,
    pub(crate) cwd_exists: bool,
    /// Unix seconds the file was last written.
    pub(crate) modified: i64,
    pub(crate) bytes: u64,
    /// The Forge session an earlier import made of it, while that session still exists.
    pub(crate) imported_session_id: Option<String>,
    #[serde(skip)]
    pub(crate) path: PathBuf,
}

/// What importing one candidate did.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Outcome {
    pub(crate) agent: Agent,
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) session_id: Option<String>,
    pub(crate) messages: usize,
    /// Already in Forge: nothing was written.
    pub(crate) skipped: bool,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Selection {
    pub(crate) agent: Agent,
    pub(crate) id: String,
}

fn secs(t: std::time::SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn first_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|b| (b["type"] == "text").then(|| b["text"].as_str()).flatten())
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let t = text.trim();
    (!t.is_empty() && !t.starts_with('<')).then(|| t.to_string())
}

/// `(cwd, title, first prompt)` from the start of a Claude Code transcript.
fn claude_head(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (None, None, None);
    };
    let mut reader = BufReader::new(file.take(HEAD_BYTES));
    let (mut cwd, mut title, mut prompt) = (None, None, None);
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap_or(0) > 0 {
        if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
            if v["isSidechain"] != true {
                if cwd.is_none() {
                    cwd = v["cwd"]
                        .as_str()
                        .filter(|c| !c.is_empty())
                        .map(str::to_string);
                }
                for key in ["customTitle", "aiTitle"] {
                    if let Some(t) = v[key].as_str().filter(|t| !t.trim().is_empty()) {
                        title = Some(t.to_string());
                    }
                }
                if prompt.is_none() && v["type"] == "user" && v["isMeta"] != true {
                    prompt = first_text(&v["message"]["content"]);
                }
            }
        }
        line.clear();
    }
    (cwd, title, prompt)
}

fn scan_claude(projects: &Path, out: &mut Vec<Candidate>) {
    for dir in std::fs::read_dir(projects).into_iter().flatten().flatten() {
        for entry in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .flatten()
        {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Some(id) = super::switch::cc_fs::session_id_of(&path) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            if !super::switch::cc_fs::is_interactive(&path) {
                continue;
            }
            let (cwd, title, prompt) = claude_head(&path);
            let Some(title) = title.or(prompt).as_deref().and_then(clean_title) else {
                continue;
            };
            out.push(Candidate {
                agent: Agent::Claude,
                id,
                title,
                cwd_exists: cwd.as_deref().is_some_and(|c| Path::new(c).is_dir()),
                cwd,
                modified: meta.modified().map_or(0, secs),
                bytes: meta.len(),
                imported_session_id: None,
                path,
            });
        }
    }
}

fn scan_codex(dir: &Path, out: &mut Vec<Candidate>) {
    for path in codex_rollout::rollout_files(dir) {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let head = codex_rollout::head(&path);
        let (Some(id), Some(title)) = (head.id, head.first_prompt.as_deref().and_then(clean_title))
        else {
            continue;
        };
        out.push(Candidate {
            agent: Agent::Codex,
            id,
            title,
            cwd_exists: head.cwd.as_deref().is_some_and(|c| Path::new(c).is_dir()),
            cwd: head.cwd,
            modified: meta.modified().map_or(0, secs),
            bytes: meta.len(),
            imported_session_id: None,
            path,
        });
    }
}

/// Codex has no cross-link table like Claude's, so what was imported is remembered in a small
/// file beside the database it was imported into.
struct Ledger {
    path: PathBuf,
    map: BTreeMap<String, String>,
}

impl Ledger {
    fn open(store: &Store) -> Option<Self> {
        let db = store.db_path()?;
        let mut name = db.file_name()?.to_os_string();
        name.push(".codex-imports.json");
        let path = db.with_file_name(name);
        let map = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Some(Self { path, map })
    }

    fn record(&mut self, codex_id: &str, session_id: &str) {
        self.map
            .insert(codex_id.to_string(), session_id.to_string());
        let tmp = self.path.with_extension("tmp");
        if let Ok(text) = serde_json::to_string_pretty(&self.map) {
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }
}

/// Everything importable on this machine, newest first. `store` marks what is already in Forge.
pub(crate) fn scan(store: Option<&Store>) -> Vec<Candidate> {
    scan_in(
        super::switch::cc_fs::projects_dir().ok().as_deref(),
        codex_rollout::sessions_dir().as_deref(),
        store,
    )
}

/// [`scan`] over explicit Claude Code `projects` and Codex `sessions` directories.
pub(crate) fn scan_in(
    claude_projects: Option<&Path>,
    codex_sessions: Option<&Path>,
    store: Option<&Store>,
) -> Vec<Candidate> {
    let mut found = Vec::new();
    if let Some(dir) = claude_projects {
        scan_claude(dir, &mut found);
    }
    if let Some(dir) = codex_sessions {
        scan_codex(dir, &mut found);
    }
    if let Some(store) = store {
        let ledger = Ledger::open(store);
        for c in &mut found {
            let session = match c.agent {
                Agent::Claude => store
                    .cc_link_for_cc(&c.id)
                    .ok()
                    .flatten()
                    .map(|l| l.forge_session_id),
                Agent::Codex => ledger.as_ref().and_then(|l| l.map.get(&c.id).cloned()),
            };
            c.imported_session_id = session.filter(|s| store.session_exists(s).unwrap_or(false));
        }
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    found
}

fn pin_codex(model: Option<&str>) -> Option<String> {
    let raw = model?.trim();
    (!raw.is_empty()).then(|| {
        if raw.contains("::") {
            raw.to_string()
        } else {
            format!("{CODEX_PREFIX}{raw}")
        }
    })
}

/// Import one Codex rollout as a new Forge session; returns its id and how many messages it holds.
pub(crate) fn import_rollout(
    store: &Store,
    path: &Path,
    cwd: Option<&Path>,
    title: Option<&str>,
    mode: &str,
) -> Result<(String, usize)> {
    let rollout = codex_rollout::read(path)?;
    if !rollout
        .messages
        .iter()
        .any(|m| matches!(m.role, Role::User | Role::Assistant))
    {
        bail!(
            "{} holds no conversation (no user or assistant messages)",
            path.display()
        );
    }
    let cwd = match (cwd, &rollout.meta.cwd) {
        (Some(c), _) => c.to_path_buf(),
        (None, Some(c)) => PathBuf::from(c),
        (None, None) => std::env::current_dir().context("no working directory")?,
    }
    .display()
    .to_string();
    let title = title
        .and_then(clean_title)
        .or_else(|| rollout.meta.first_prompt.as_deref().and_then(clean_title));
    let model = pin_codex(rollout.meta.model.as_deref());
    let count = rollout.messages.len();
    let session_id = store.import_session_linked_with(
        &ImportedSessionMeta {
            cwd: &cwd,
            mode,
            title: title.as_deref(),
            pinned_model: model.as_deref(),
        },
        None,
        |sink| {
            for m in rollout.messages {
                sink.push(ImportedRow {
                    role: m.role,
                    content: m.content,
                    model: m.model,
                    tool_calls: m.tool_calls,
                    tool_call_id: m.tool_call_id,
                    active: m.live,
                    created_at: m.timestamp,
                })?;
            }
            Ok(None)
        },
    )?;
    Ok((session_id, count))
}

/// The Forge permission mode a new import is stored with.
pub(crate) fn default_mode() -> String {
    let mode = forge_config::load()
        .map(|c| c.permission_mode)
        .unwrap_or(forge_types::PermissionMode::Default);
    format!("{mode:?}")
}

/// Import `picks` from a fresh scan. A conversation whose directory is gone here lands in
/// `fallback_cwd` (else the home directory) so the new session can still be opened.
pub(crate) fn import_selected(
    store: &Store,
    picks: &[Selection],
    fallback_cwd: Option<&Path>,
    mode: &str,
) -> Vec<Outcome> {
    import_from(store, &scan(Some(store)), picks, fallback_cwd, mode)
}

/// [`import_selected`] over an already-scanned list.
pub(crate) fn import_from(
    store: &Store,
    candidates: &[Candidate],
    picks: &[Selection],
    fallback_cwd: Option<&Path>,
    mode: &str,
) -> Vec<Outcome> {
    let mut ledger = Ledger::open(store);
    picks
        .iter()
        .map(|pick| {
            let Some(c) = candidates
                .iter()
                .find(|c| c.agent == pick.agent && c.id == pick.id)
            else {
                return Outcome {
                    agent: pick.agent,
                    id: pick.id.clone(),
                    title: String::new(),
                    session_id: None,
                    messages: 0,
                    skipped: false,
                    error: Some("not found on this machine".into()),
                };
            };
            let mut out = Outcome {
                agent: c.agent,
                id: c.id.clone(),
                title: c.title.clone(),
                session_id: c.imported_session_id.clone(),
                messages: 0,
                skipped: c.imported_session_id.is_some(),
                error: None,
            };
            if out.skipped {
                return out;
            }
            let cwd: Option<PathBuf> = if c.cwd_exists {
                None
            } else {
                Some(
                    fallback_cwd
                        .map(Path::to_path_buf)
                        .or_else(forge_config::home_dir)
                        .unwrap_or_else(|| PathBuf::from("/")),
                )
            };
            let done = match c.agent {
                Agent::Claude => import_transcript(
                    store,
                    &c.path,
                    &ClaudeSessionArgs {
                        session: c.path.display().to_string(),
                        model: None,
                        title: None,
                        cwd,
                        quiet: true,
                    },
                    mode,
                )
                .map(|r| (r.session_id, r.stats.messages)),
                Agent::Codex => import_rollout(store, &c.path, cwd.as_deref(), None, mode),
            };
            match done {
                Ok((session_id, messages)) => {
                    if c.agent == Agent::Codex {
                        if let Some(l) = ledger.as_mut() {
                            l.record(&c.id, &session_id);
                        }
                    }
                    out.session_id = Some(session_id);
                    out.messages = messages;
                }
                Err(e) => out.error = Some(format!("{e:#}")),
            }
            out
        })
        .collect()
}

pub(crate) struct CodexSessionArgs {
    pub(crate) session: String,
    pub(crate) title: Option<String>,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) quiet: bool,
}

/// `forge import codex-session <id|path>`: prints the new session id on stdout.
pub(crate) fn import_codex_session_cmd(args: CodexSessionArgs) -> Result<()> {
    let store = crate::open_store()?;
    let path = codex_rollout::locate(&args.session)?;
    let (session_id, messages) = import_rollout(
        &store,
        &path,
        args.cwd.as_deref(),
        args.title.as_deref(),
        &default_mode(),
    )?;
    if let Some(mut ledger) = Ledger::open(&store) {
        if let Some(id) = codex_rollout::head(&path).id {
            ledger.record(&id, &session_id);
        }
    }
    if !args.quiet {
        eprintln!(
            "imported {} ({messages} messages)\n  open it with: forge chat --resume {session_id}",
            path.display()
        );
    }
    println!("{session_id}");
    Ok(())
}

fn print_row(c: &Candidate) {
    println!(
        "{:<7}{:<38}{}{}  {}",
        c.agent.label(),
        c.id,
        if c.imported_session_id.is_some() {
            "imported  "
        } else {
            ""
        },
        c.title,
        c.cwd.as_deref().unwrap_or("(no directory)"),
    );
}

/// `forge import scan [--agent A] [--json]`.
pub(crate) fn scan_cmd(agent: Option<Agent>, json: bool) -> Result<()> {
    let store = crate::open_store()?;
    let found: Vec<Candidate> = scan(Some(&store))
        .into_iter()
        .filter(|c| agent.is_none_or(|a| a == c.agent))
        .collect();
    if json {
        println!("{}", serde_json::to_string_pretty(&found)?);
    } else if found.is_empty() {
        println!("no importable sessions found on this computer");
    } else {
        found.iter().for_each(print_row);
    }
    Ok(())
}

/// `forge import sessions <id>... | --all [--agent A] [--cwd DIR]`.
pub(crate) fn sessions_cmd(
    ids: &[String],
    all: bool,
    agent: Option<Agent>,
    cwd: Option<&Path>,
) -> Result<()> {
    let store = crate::open_store()?;
    let found = scan(Some(&store));
    let picks: Vec<Selection> = if all {
        found
            .iter()
            .filter(|c| c.imported_session_id.is_none() && agent.is_none_or(|a| a == c.agent))
            .map(|c| Selection {
                agent: c.agent,
                id: c.id.clone(),
            })
            .collect()
    } else {
        let mut picks = Vec::new();
        for id in ids {
            let hits: Vec<&Candidate> = found
                .iter()
                .filter(|c| c.id == *id && agent.is_none_or(|a| a == c.agent))
                .collect();
            match hits.as_slice() {
                [c] => picks.push(Selection {
                    agent: c.agent,
                    id: c.id.clone(),
                }),
                [] => bail!("no importable session {id}; list them with `forge import scan`"),
                _ => bail!("{id} matches more than one agent; add --agent"),
            }
        }
        picks
    };
    if picks.is_empty() {
        println!("nothing to import");
        return Ok(());
    }
    let outcomes = import_selected(&store, &picks, cwd, &default_mode());
    let failed = outcomes.iter().filter(|o| o.error.is_some()).count();
    for o in &outcomes {
        match (&o.error, o.skipped, &o.session_id) {
            (Some(e), _, _) => println!("failed    {} {}: {e}", o.agent.label(), o.id),
            (None, true, Some(s)) => {
                println!("skipped   {} {} (already {s})", o.agent.label(), o.id)
            }
            (None, _, Some(s)) => {
                println!("imported  {} {} -> {s}  {}", o.agent.label(), o.id, o.title)
            }
            _ => {}
        }
    }
    if failed > 0 {
        bail!("{failed} of {} sessions failed to import", outcomes.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests;

impl From<super::super::args::import::ImportAgent> for Agent {
    fn from(a: super::super::args::import::ImportAgent) -> Self {
        match a {
            super::super::args::import::ImportAgent::Claude => Agent::Claude,
            super::super::args::import::ImportAgent::Codex => Agent::Codex,
        }
    }
}
