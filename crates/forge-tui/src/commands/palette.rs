//! The slash palette, the command center and the `@path` picker: the filterable state behind the
//! composer's completion popups. Pure data; `app::render::popups` draws it.

use super::{command_category, Command, COMMANDS};

/// One row of the `@path` picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AtEntry {
    /// Workspace-relative path; directories end in `/`.
    pub path: String,
    pub is_dir: bool,
    /// A file (or, for a directory, something under it) that git reports as changed.
    pub modified: bool,
    /// Files under a directory.
    pub items: usize,
}

/// Inline file-path picker state. Opens when the input line contains an `@path` token at cursor.
/// An empty query browses the top level (directories first); `dir/` browses that directory; any
/// other query is a fuzzy search over every file in the workspace.
#[derive(Debug, Clone, Default)]
pub struct AtPathPicker {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub anim: f32,
    files: Vec<String>,
    modified: std::collections::HashSet<String>,
}

/// Rank of `path` for `query` (lower is better): basename prefix, basename substring, path
/// substring, then subsequence. `None` when it does not match at all.
fn at_score(path: &str, query: &str) -> Option<u32> {
    let lower = path.to_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    if base.starts_with(query) {
        Some(0)
    } else if base.contains(query) {
        Some(1)
    } else if lower.contains(query) {
        Some(2)
    } else {
        let mut q = query.chars().peekable();
        for c in lower.chars() {
            if q.peek() == Some(&c) {
                q.next();
            }
        }
        q.peek().is_none().then_some(3)
    }
}

impl AtPathPicker {
    pub fn open_with(&mut self, query: &str, files: Vec<String>) {
        self.open = true;
        self.query = query.to_string();
        self.files = files;
        self.selected = 0;
        self.anim = 0.0;
    }

    /// Files git reports as changed, so the picker can flag them.
    pub fn set_modified(&mut self, files: impl IntoIterator<Item = String>) {
        self.modified = files.into_iter().collect();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.anim = 0.0;
    }

    /// Rows for the current query, in display order.
    pub fn entries(&self) -> Vec<AtEntry> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() || query.ends_with('/') {
            return self.browse(&self.query.trim().replace('\\', "/"));
        }
        let mut scored: Vec<(u32, usize, &String)> = self
            .files
            .iter()
            .filter_map(|f| at_score(f, &query).map(|s| (s, f.len(), f)))
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        scored
            .into_iter()
            .map(|(_, _, f)| AtEntry {
                path: f.clone(),
                is_dir: false,
                modified: self.modified.contains(f),
                items: 0,
            })
            .collect()
    }

    fn browse(&self, prefix: &str) -> Vec<AtEntry> {
        let mut dirs: Vec<AtEntry> = Vec::new();
        let mut files: Vec<AtEntry> = Vec::new();
        for f in self.files.iter().filter(|f| f.starts_with(prefix)) {
            let rest = &f[prefix.len()..];
            match rest.split_once('/') {
                Some((dir, _)) => {
                    let path = format!("{prefix}{dir}/");
                    match dirs.iter_mut().find(|d| d.path == path) {
                        Some(d) => {
                            d.items += 1;
                            d.modified |= self.modified.contains(f);
                        }
                        None => dirs.push(AtEntry {
                            path,
                            is_dir: true,
                            modified: self.modified.contains(f),
                            items: 1,
                        }),
                    }
                }
                None if !rest.is_empty() => files.push(AtEntry {
                    path: f.clone(),
                    is_dir: false,
                    modified: self.modified.contains(f),
                    items: 0,
                }),
                None => {}
            }
        }
        dirs.sort_by(|a, b| a.path.cmp(&b.path));
        files.sort_by(|a, b| a.path.cmp(&b.path));
        dirs.extend(files);
        dirs
    }

    /// The paths of [`Self::entries`] (directories end in `/`).
    pub fn matches(&self) -> Vec<String> {
        self.entries().into_iter().map(|e| e.path).collect()
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jump `rows` up (PgUp).
    pub fn page_up(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
    }

    /// Jump `rows` down (PgDn).
    pub fn page_down(&mut self, rows: usize) {
        let last = self.entries().len().saturating_sub(1);
        self.selected = (self.selected + rows).min(last);
    }

    pub fn move_down(&mut self) {
        let n = self.entries().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    pub fn clamp(&mut self) {
        let n = self.entries().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    pub fn selected_path(&self) -> Option<String> {
        self.matches().into_iter().nth(self.selected)
    }

    pub fn tick_anim(&mut self) {
        if self.open && self.anim < 1.0 {
            self.anim = (self.anim + 0.34).min(1.0);
        }
    }
}

/// Rank a command against a query: `Some(score)` if it matches (lower = better), `None` if not.
/// Prefix matches rank above subsequence (fuzzy) matches; an empty query matches everything.
fn match_score(name: &str, query: &str) -> Option<u32> {
    if query.is_empty() {
        return Some(1000); // stable order preserved by the caller's enumerate tiebreak
    }
    let (name, query) = (name.to_lowercase(), query.to_lowercase());
    if name.starts_with(&query) {
        return Some(query.len() as u32); // exact-prefix: best, shorter query already typed = closer
    }
    // Subsequence fuzzy: every query char appears in order.
    let mut q = query.chars().peekable();
    for c in name.chars() {
        if q.peek() == Some(&c) {
            q.next();
        }
    }
    if q.peek().is_none() {
        Some(500 + name.len() as u32) // matched, but worse than any prefix hit
    } else {
        None
    }
}

/// Commands matching `query`, best-first (prefix before fuzzy, then registry order).
pub fn filter_commands(query: &str) -> Vec<&'static Command> {
    let mut scored: Vec<(u32, usize, &'static Command)> = COMMANDS
        .iter()
        .enumerate()
        .filter_map(|(i, c)| match_score(c.name, query).map(|s| (s, i, c)))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

/// One palette row, owned so it can mix the static builtins with file-based commands/skills
/// (the binary populates [`Palette::extra`] from `forge_skills::Catalog`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub name: String,
    pub desc: String,
    /// Usage/argument hint (e.g. `/effort [low|medium|high|xhigh]`). Empty for file-based commands
    /// and skills that take freeform input; shown for the highlighted row so non-obvious args
    /// (`/assay`, `/replay`, `/model`) are discoverable inline.
    pub usage: String,
}

/// A categorized command row used by the global command center. The slash palette stays focused
/// on text completion; this surface makes every built-in, skill, and custom command discoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandCenterEntry {
    pub name: String,
    pub desc: String,
    pub usage: String,
    pub category: &'static str,
}

fn command_category_rank(category: &str) -> usize {
    match category {
        "Start work" => 0,
        "Session" => 1,
        "Model & usage" => 2,
        "Review & ship" => 3,
        "Integrations" => 4,
        "Settings & help" => 5,
        "Utilities" => 6,
        "Skills & custom" => 7,
        _ => 8,
    }
}

/// Interaction state for the global searchable command center. Command execution remains routed
/// through the existing slash-command parser and CLI dispatch layer.
#[derive(Debug, Clone, Default)]
pub struct CommandCenter {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    /// Active group tab (index into [`COMMAND_GROUPS`]); 0 shows everything.
    pub group: usize,
}

/// The command center's group tabs. `Tab` cycles them; `All` shows every entry.
pub const COMMAND_GROUPS: [&str; 7] = [
    "All", "Start", "Session", "Model", "Tools", "Skills", "Settings",
];

/// Which group tab a command category belongs to (never 0: `All` is not a category).
fn category_group(category: &str) -> usize {
    match category {
        "Start work" => 1,
        "Session" => 2,
        "Model & usage" => 3,
        "Skills & custom" => 5,
        "Settings & help" => 6,
        _ => 4,
    }
}

impl CommandCenter {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.group = 0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.group = 0;
    }

    /// Move to the next group tab (wrapping) and restart the selection.
    pub fn cycle_group(&mut self) {
        self.group = (self.group + 1) % COMMAND_GROUPS.len();
        self.selected = 0;
    }

    pub fn matches(&self, extra: &[PaletteEntry]) -> Vec<CommandCenterEntry> {
        let mut entries: Vec<CommandCenterEntry> = COMMANDS
            .iter()
            .map(|command| CommandCenterEntry {
                name: command.name.to_string(),
                desc: command.desc.to_string(),
                usage: command.usage.to_string(),
                category: command_category(command.name),
            })
            .collect();
        entries.extend(extra.iter().map(|entry| CommandCenterEntry {
            name: entry.name.clone(),
            desc: entry.desc.clone(),
            usage: entry.usage.clone(),
            category: "Skills & custom",
        }));

        let query = self.query.to_lowercase();
        let empty_query = query.is_empty();
        let mut scored: Vec<(u32, usize, CommandCenterEntry)> = entries
            .into_iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let name_score = match_score(&entry.name, &query);
                let description_score = match_score(&entry.desc, &query).map(|score| score + 250);
                name_score
                    .into_iter()
                    .chain(description_score)
                    .min()
                    .map(|score| (score, index, entry))
            })
            .collect();
        scored.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| {
                    if empty_query {
                        command_category_rank(a.2.category)
                            .cmp(&command_category_rank(b.2.category))
                    } else {
                        a.1.cmp(&b.1)
                    }
                })
                .then(a.1.cmp(&b.1))
        });
        scored
            .into_iter()
            .map(|(_, _, entry)| entry)
            .filter(|e| self.group == 0 || category_group(e.category) == self.group)
            .collect()
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jump `rows` up (PgUp), or to the first entry (`None`, Home).
    pub fn jump_up(&mut self, rows: Option<usize>) {
        self.selected = rows.map_or(0, |n| self.selected.saturating_sub(n));
    }

    /// Jump `rows` down (PgDn), or to the last entry (`None`, End).
    pub fn jump_down(&mut self, rows: Option<usize>, extra: &[PaletteEntry]) {
        let last = self.matches(extra).len().saturating_sub(1);
        self.selected = rows.map_or(last, |n| (self.selected + n).min(last));
    }

    pub fn move_down(&mut self, extra: &[PaletteEntry]) {
        let count = self.matches(extra).len();
        if count > 0 {
            self.selected = (self.selected + 1).min(count - 1);
        }
    }

    pub fn clamp(&mut self, extra: &[PaletteEntry]) {
        let count = self.matches(extra).len();
        if count == 0 {
            self.selected = 0;
        } else if self.selected >= count {
            self.selected = count - 1;
        }
    }

    pub fn selected_entry(&self, extra: &[PaletteEntry]) -> Option<CommandCenterEntry> {
        self.matches(extra).into_iter().nth(self.selected)
    }
}

/// Inline command-palette state. Opens when the input line starts with `/`. The binary's render
/// loop drives it via keystrokes and reads the filtered list / selection.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    pub open: bool,
    /// Text after the leading `/` (the filter query).
    pub query: String,
    pub selected: usize,
    /// Eases 0.0 → 1.0 on open for the reveal animation (advanced by the render tick).
    pub anim: f32,
    /// File-based commands + skills discovered at startup, shown alongside the builtins. Set
    /// once by the binary; preserved across open/close.
    pub extra: Vec<PaletteEntry>,
    /// Command names run most recently, newest first. Listed first on an empty query.
    pub recents: Vec<String>,
}

impl Palette {
    pub fn open_with(&mut self, query: &str) {
        self.open = true;
        self.query = query.to_string();
        self.selected = 0;
        self.anim = 0.0;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = 0;
        self.anim = 0.0;
    }

    /// The currently-filtered palette rows: builtins + any file-based `extra`, ranked best-first
    /// (prefix before fuzzy, then order), deduped by name (a builtin wins a name clash).
    pub fn matches(&self) -> Vec<PaletteEntry> {
        let mut all: Vec<PaletteEntry> = COMMANDS
            .iter()
            .map(|c| PaletteEntry {
                name: c.name.to_string(),
                desc: c.desc.to_string(),
                usage: c.usage.to_string(),
            })
            .collect();
        all.extend(self.extra.iter().cloned());
        let mut scored: Vec<(u32, usize, PaletteEntry)> = all
            .into_iter()
            .enumerate()
            .filter_map(|(i, e)| match_score(&e.name, &self.query).map(|s| (s, i, e)))
            .collect();
        scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut seen = std::collections::HashSet::new();
        let mut ranked: Vec<PaletteEntry> = scored
            .into_iter()
            .filter(|(_, _, e)| seen.insert(e.name.clone()))
            .map(|(_, _, e)| e)
            .collect();
        if self.query.is_empty() && !self.recents.is_empty() {
            let rank = |name: &str| self.recents.iter().position(|r| r == name);
            // Stable: recents move to the front in recency order, the rest keep registry order.
            ranked.sort_by_key(|e| rank(&e.name).unwrap_or(usize::MAX));
        }
        ranked
    }

    /// How many leading rows are recents (empty query only).
    pub fn recent_count(&self) -> usize {
        if !self.query.is_empty() {
            return 0;
        }
        self.matches()
            .iter()
            .take_while(|e| self.recents.contains(&e.name))
            .count()
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jump `rows` up (PgUp).
    pub fn page_up(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
    }

    /// Jump `rows` down (PgDn).
    pub fn page_down(&mut self, rows: usize) {
        let last = self.matches().len().saturating_sub(1);
        self.selected = (self.selected + rows).min(last);
    }

    pub fn move_down(&mut self) {
        let n = self.matches().len();
        if n > 0 {
            self.selected = (self.selected + 1).min(n - 1);
        }
    }

    /// Clamp the selection after the list shrinks (query got more specific).
    pub fn clamp(&mut self) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// The selected row's name, for Tab-completion / Enter.
    pub fn selected_name(&self) -> Option<String> {
        self.matches()
            .into_iter()
            .nth(self.selected)
            .map(|e| e.name)
    }

    /// The highlighted row's usage/argument hint, shown inline in the palette. Empty when the
    /// command has no non-obvious args.
    pub fn selected_usage(&self) -> Option<String> {
        self.matches()
            .into_iter()
            .nth(self.selected)
            .map(|e| e.usage)
            .filter(|u| !u.is_empty())
    }

    /// Advance the reveal animation toward 1.0 (called per render tick while open).
    pub fn tick_anim(&mut self) {
        if self.open && self.anim < 1.0 {
            self.anim = (self.anim + 0.34).min(1.0);
        }
    }
}

/// The keybind action that does what `/name` does, so lists can show its shortcut.
pub fn command_action(name: &str) -> Option<&'static str> {
    match name {
        "help" => Some("help"),
        "keys" => Some("keybind_config"),
        "mode" => Some("temper_cycle"),
        "model" | "models" => Some("model_picker"),
        "effort" => Some("effort_cycle"),
        "compact" => Some("compact"),
        "new" => Some("new_session"),
        "checkpoint" => Some("checkpoint"),
        "undo" => Some("undo"),
        "copy" => Some("copy_last"),
        "voice" => Some("voice"),
        "thinking" => Some("toggle_reasoning"),
        _ => None,
    }
}

/// Best-effort fixed argument values for a command, parsed from a `[a|b|c]` group in its usage
/// string (e.g. `/effort [low|medium|high|xhigh]` → `["low","medium","high","xhigh"]`). Returns
/// empty for commands whose args are freeform (paths, ids, prompts). Used to offer enum-value
/// completion once the user has typed the command word + a space.
pub fn arg_values(name: &str) -> Vec<&'static str> {
    let usage = match COMMANDS.iter().find(|c| c.name == name) {
        Some(c) => c.usage,
        None => return Vec::new(),
    };
    let (open, close) = match (usage.find('['), usage.find(']')) {
        (Some(o), Some(c)) if c > o + 1 => (o, c),
        _ => return Vec::new(),
    };
    let inner = &usage[open + 1..close];
    // Only treat it as a fixed enum when it's a clean `a|b|c` set (no spaces/sub-args like
    // `[name]` or `[--flag <v>]`).
    if !inner.contains('|') || inner.contains(' ') || inner.contains('<') {
        return Vec::new();
    }
    inner
        .split('|')
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect()
}
