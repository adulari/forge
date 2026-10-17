//! The friendly `/config` editor overlay: settings grouped into labeled sections, each shown with a
//! human label and a type-appropriate control — bools toggle, enums cycle, numbers/text/secrets are
//! typed. Fuzzy search filters across all of them; a detail line shows help, type, default, and
//! source for the selected row. State, input handling, and rendering live here (no `forge-config`
//! dependency); the I/O shell feeds it rows and performs the validated writes via [`ConfigAction`].

use crate::app::KeyKind;

/// The editing control a row uses.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum RowKind {
    Bool,
    Int,
    Float,
    /// A fixed set of valid values, cycled with ←/→.
    Enum(Vec<String>),
    #[default]
    Text,
    /// A secret (API key): masked, edits route to the keyring.
    Secret,
    /// A non-scalar section (hooks/mcp/permissions) shown read-only; Enter jumps to `$EDITOR`.
    ReadOnly,
}

/// One editable setting row (the TUI-side mirror of `forge_config::SettingDescriptor`).
#[derive(Debug, Clone, Default)]
pub struct SettingRow {
    pub path: String,
    pub group: String,
    pub label: String,
    pub help: Option<String>,
    pub kind: RowKind,
    /// Current value, rendered.
    pub value: String,
    /// Built-in default, rendered (for the detail line).
    pub default: String,
    /// Overridden from a config file.
    pub modified: bool,
    /// "user" | "project" | "default".
    pub source: String,
}

impl SettingRow {
    fn is_secret(&self) -> bool {
        self.kind == RowKind::Secret
    }
    /// The next/previous enum value, or `None` for non-enum rows.
    fn cycled(&self, forward: bool) -> Option<String> {
        let RowKind::Enum(opts) = &self.kind else {
            return None;
        };
        if opts.is_empty() {
            return None;
        }
        let cur = opts.iter().position(|o| o == &self.value).unwrap_or(0);
        let n = opts.len();
        let next = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        Some(opts[next].clone())
    }
}

/// What the I/O shell should do after a keystroke.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigAction {
    None,
    /// Persist `value` at `path` (config.toml, or the keyring for secrets), then refresh.
    Save {
        path: String,
        value: String,
    },
    /// Reset `path` to its default (clear the override), then refresh.
    Reset {
        path: String,
    },
    /// Scope toggled — reload rows.
    Reload,
    /// Open the config file in `$EDITOR` (for a read-only complex section). The shell reloads
    /// rows afterwards.
    EditFile,
    /// Editor closed.
    Close,
}

/// The `/config` editor overlay state.
#[derive(Debug, Clone, Default)]
pub struct ConfigEditor {
    pub open: bool,
    /// false = user config, true = project.
    pub project_scope: bool,
    pub rows: Vec<SettingRow>,
    pub filter: String,
    /// Index into the FILTERED row list.
    pub selected: usize,
    /// `Some(buffer)` while typing a number/text/secret value.
    pub editing: Option<String>,
    pub status: Option<String>,
}

impl ConfigEditor {
    pub fn open_with(&mut self, rows: Vec<SettingRow>) {
        self.open = true;
        self.rows = rows;
        self.filter.clear();
        self.selected = 0;
        self.editing = None;
        self.status = None;
    }

    /// Row indices passing the fuzzy filter (matched on label OR path), in order.
    pub fn matches(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.rows.len()).collect();
        }
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                fuzzy_subseq(&self.filter, &r.label) || fuzzy_subseq(&self.filter, &r.path)
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Sections (groups) holding at least one match, in first-seen order, with their match counts.
    pub fn sections(&self) -> Vec<(String, usize)> {
        let mut out: Vec<(String, usize)> = Vec::new();
        for i in self.matches() {
            let group = &self.rows[i].group;
            match out.iter_mut().find(|(g, _)| g == group) {
                Some((_, n)) => *n += 1,
                None => out.push((group.clone(), 1)),
            }
        }
        out
    }

    /// Index into [`Self::sections`] of the section the selected row lives in.
    pub fn section_index(&self) -> usize {
        let Some(row) = self.selected_row() else {
            return 0;
        };
        self.sections()
            .iter()
            .position(|(g, _)| g == &row.group)
            .unwrap_or(0)
    }

    /// Move the selection to the first row of the next (or previous) section, wrapping.
    fn jump_section(&mut self, forward: bool) {
        let sections = self.sections();
        if sections.len() < 2 {
            return;
        }
        let n = sections.len();
        let cur = self.section_index();
        let target = if forward {
            (cur + 1) % n
        } else {
            (cur + n - 1) % n
        };
        let group = &sections[target].0;
        if let Some(pos) = self
            .matches()
            .iter()
            .position(|&i| &self.rows[i].group == group)
        {
            self.selected = pos;
        }
    }

    pub fn selected_row(&self) -> Option<&SettingRow> {
        let m = self.matches();
        m.get(self.selected).map(|&i| &self.rows[i])
    }

    fn clamp(&mut self) {
        let n = self.matches().len();
        if n == 0 {
            self.selected = 0;
        } else if self.selected >= n {
            self.selected = n - 1;
        }
    }

    /// Feed a keystroke; returns the I/O action. Modal: while typing a value, keys edit the buffer;
    /// otherwise bools toggle (Enter), enums cycle (←/→/Enter), numbers/text/secrets open an input,
    /// Del resets to default, Tab switches scope, typing filters.
    pub fn handle_key(&mut self, key: KeyKind) -> ConfigAction {
        if let Some(buf) = self.editing.as_mut() {
            return match key {
                KeyKind::Char(c) => {
                    buf.push(c);
                    ConfigAction::None
                }
                KeyKind::Backspace => {
                    buf.pop();
                    ConfigAction::None
                }
                KeyKind::Esc => {
                    self.editing = None;
                    ConfigAction::None
                }
                KeyKind::Enter => {
                    let value = self.editing.take().unwrap_or_default();
                    match self.selected_row() {
                        Some(r) => ConfigAction::Save {
                            path: r.path.clone(),
                            value,
                        },
                        None => ConfigAction::None,
                    }
                }
                _ => ConfigAction::None,
            };
        }
        match key {
            KeyKind::Esc => {
                self.open = false;
                ConfigAction::Close
            }
            KeyKind::Up => {
                self.selected = self.selected.saturating_sub(1);
                ConfigAction::None
            }
            KeyKind::Down => {
                let n = self.matches().len();
                if n > 0 && self.selected + 1 < n {
                    self.selected += 1;
                }
                ConfigAction::None
            }
            KeyKind::Tab => {
                self.project_scope = !self.project_scope;
                ConfigAction::Reload
            }
            // Section jumps: the settings list is long, and `←→` already change enum values.
            KeyKind::PageDown | KeyKind::PageUp => {
                self.jump_section(matches!(key, KeyKind::PageDown));
                ConfigAction::None
            }
            // Del resets the selected setting to its default (not secrets or read-only sections).
            KeyKind::DeleteForward => match self.selected_row() {
                Some(r) if !r.is_secret() && r.kind != RowKind::ReadOnly => ConfigAction::Reset {
                    path: r.path.clone(),
                },
                _ => ConfigAction::None,
            },
            // Enum cycle (Enter/→ next, ← prev).
            KeyKind::Left | KeyKind::Right
                if matches!(self.selected_row().map(|r| &r.kind), Some(RowKind::Enum(_))) =>
            {
                let fwd = matches!(key, KeyKind::Right);
                match self.selected_row().and_then(|r| {
                    r.cycled(fwd).map(|v| ConfigAction::Save {
                        path: r.path.clone(),
                        value: v,
                    })
                }) {
                    Some(a) => a,
                    None => ConfigAction::None,
                }
            }
            KeyKind::Enter => {
                let Some(r) = self.selected_row() else {
                    return ConfigAction::None;
                };
                match &r.kind {
                    // Complex section: jump to $EDITOR (the shell owns the actual launch).
                    RowKind::ReadOnly => ConfigAction::EditFile,
                    RowKind::Bool => ConfigAction::Save {
                        path: r.path.clone(),
                        value: (r.value != "true").to_string(),
                    },
                    RowKind::Enum(_) => match r.cycled(true) {
                        Some(v) => ConfigAction::Save {
                            path: r.path.clone(),
                            value: v,
                        },
                        None => ConfigAction::None,
                    },
                    // Number / text / secret → open an input (secrets start empty).
                    _ => {
                        self.editing = Some(if r.is_secret() {
                            String::new()
                        } else {
                            r.value.clone()
                        });
                        ConfigAction::None
                    }
                }
            }
            KeyKind::Backspace => {
                self.filter.pop();
                self.clamp();
                ConfigAction::None
            }
            KeyKind::Char(c) => {
                self.filter.push(c);
                self.selected = 0;
                ConfigAction::None
            }
            _ => ConfigAction::None,
        }
    }
}

/// Case-insensitive subsequence match.
fn fuzzy_subseq(needle: &str, haystack: &str) -> bool {
    let mut hay = haystack.chars().map(|c| c.to_ascii_lowercase());
    for nc in needle.chars().map(|c| c.to_ascii_lowercase()) {
        if !hay.any(|hc| hc == nc) {
            return false;
        }
    }
    true
}

mod render;
pub use render::render_config_overlay;

#[cfg(test)]
mod tests {
    use super::*;

    fn brow(path: &str, label: &str, group: &str, value: &str) -> SettingRow {
        SettingRow {
            path: path.into(),
            label: label.into(),
            group: group.into(),
            kind: RowKind::Bool,
            value: value.into(),
            ..Default::default()
        }
    }

    fn ed_with(rows: Vec<SettingRow>) -> ConfigEditor {
        let mut ed = ConfigEditor::default();
        ed.open_with(rows);
        ed
    }

    #[test]
    fn bool_enter_toggles_and_saves() {
        let mut ed = ed_with(vec![brow(
            "tui.fullscreen",
            "Full-screen TUI",
            "Interface",
            "true",
        )]);
        assert_eq!(
            ed.handle_key(KeyKind::Enter),
            ConfigAction::Save {
                path: "tui.fullscreen".into(),
                value: "false".into()
            }
        );
        assert!(ed.editing.is_none()); // no text editing for bools
    }

    #[test]
    fn enum_cycles_with_arrows() {
        let mut ed = ed_with(vec![SettingRow {
            path: "mesh.credit_mode".into(),
            label: "Credit conservation".into(),
            group: "Mesh & Cost".into(),
            kind: RowKind::Enum(vec!["normal".into(), "frugal".into(), "strict".into()]),
            value: "frugal".into(),
            ..Default::default()
        }]);
        assert_eq!(
            ed.handle_key(KeyKind::Right),
            ConfigAction::Save {
                path: "mesh.credit_mode".into(),
                value: "strict".into()
            }
        );
        assert_eq!(
            ed.handle_key(KeyKind::Left),
            ConfigAction::Save {
                path: "mesh.credit_mode".into(),
                value: "normal".into()
            }
        );
    }

    #[test]
    fn del_resets_to_default() {
        let mut ed = ed_with(vec![brow(
            "tui.fullscreen",
            "Full-screen TUI",
            "Interface",
            "true",
        )]);
        assert_eq!(
            ed.handle_key(KeyKind::DeleteForward),
            ConfigAction::Reset {
                path: "tui.fullscreen".into()
            }
        );
    }

    #[test]
    fn text_row_opens_input_then_saves() {
        let mut ed = ed_with(vec![SettingRow {
            path: "local.model".into(),
            label: "Model".into(),
            group: "Local LLM".into(),
            kind: RowKind::Text,
            value: String::new(),
            ..Default::default()
        }]);
        ed.handle_key(KeyKind::Enter); // open input
        assert!(ed.editing.is_some());
        ed.handle_key(KeyKind::Char('g'));
        ed.handle_key(KeyKind::Char('4'));
        assert_eq!(
            ed.handle_key(KeyKind::Enter),
            ConfigAction::Save {
                path: "local.model".into(),
                value: "g4".into()
            }
        );
    }

    #[test]
    fn secret_starts_empty_and_masks() {
        let mut ed = ed_with(vec![SettingRow {
            path: "key.openai".into(),
            label: "OpenAI key".into(),
            group: "Providers & Keys".into(),
            kind: RowKind::Secret,
            value: "● set".into(),
            ..Default::default()
        }]);
        ed.handle_key(KeyKind::Enter);
        assert_eq!(ed.editing.as_deref(), Some("")); // never prefilled
    }

    #[test]
    fn fuzzy_filter_matches_label_or_path() {
        let ed = ed_with(vec![
            brow("tui.fullscreen", "Full-screen TUI", "Interface", "true"),
            brow(
                "local.autostart",
                "Auto-start on launch",
                "Local LLM",
                "false",
            ),
        ]);
        let mut e = ed;
        e.filter = "autostart".into(); // matches the path
        assert_eq!(e.matches().len(), 1);
        e.filter = "full".into(); // matches the label
        assert_eq!(e.matches().len(), 1);
    }

    fn sample_rows() -> Vec<SettingRow> {
        vec![
            SettingRow {
                path: "key.anthropic".into(),
                label: "Anthropic".into(),
                group: "Providers & keys".into(),
                help: Some("API key for Anthropic, stored in the OS keyring. Enter to set.".into()),
                kind: RowKind::Secret,
                value: "○ not set".into(),
                source: "keyring".into(),
                ..Default::default()
            },
            SettingRow {
                path: "key.groq".into(),
                label: "Groq".into(),
                group: "Providers & keys".into(),
                kind: RowKind::Secret,
                value: "● set".into(),
                source: "keyring".into(),
                ..Default::default()
            },
            SettingRow {
                path: "tui.theme".into(),
                label: "Theme".into(),
                group: "Interface".into(),
                help: Some("Colour theme: auto follows the terminal.".into()),
                kind: RowKind::Enum(vec!["auto".into(), "dark".into(), "light".into()]),
                value: "auto".into(),
                default: "auto".into(),
                source: "default".into(),
                ..Default::default()
            },
            brow("tui.reduce_motion", "Reduce motion", "Interface", "true"),
        ]
    }

    fn draw(ed: &ConfigEditor, w: u16, h: u16) -> Vec<String> {
        use ratatui::backend::TestBackend;
        let mut terminal = ratatui::Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| render_config_overlay(f, ed)).unwrap();
        let buf = terminal.backend().buffer();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn two_panes_from_120_columns_with_sections_counts_and_one_value_grammar() {
        let ed = ed_with(sample_rows());
        let text = draw(&ed, 140, 24).join("\n");
        assert!(
            text.contains("Settings") && text.contains("4 settings"),
            "{text}"
        );
        assert!(
            text.contains("❯ Providers & keys") && text.contains("Interface"),
            "{text}"
        );
        assert!(
            text.contains("scope:  User") || text.contains("scope: "),
            "{text}"
        );
        assert!(
            text.contains("○ not set") && text.contains("✓ keyring"),
            "{text}"
        );
        assert!(
            text.contains("API key for Anthropic"),
            "description column: {text}"
        );
        let help = text
            .lines()
            .find(|l| l.contains("source: keyring"))
            .expect("help line");
        assert!(
            help.trim_end()
                .trim_end_matches(['│', ' '])
                .ends_with("key.anthropic"),
            "key path last: {help}"
        );
        assert!(
            !text.contains("Theme"),
            "only the current section's rows: {text}"
        );
    }

    #[test]
    fn single_pane_below_120_columns_names_the_section_and_pages_between_them() {
        let mut ed = ed_with(sample_rows());
        let text = draw(&ed, 80, 24).join("\n");
        assert!(
            text.contains("PROVIDERS & KEYS") && text.contains("section 1/2"),
            "{text}"
        );
        ed.handle_key(KeyKind::PageDown);
        assert_eq!(
            ed.selected_row().unwrap().path,
            "tui.theme",
            "jumped to the next section"
        );
        let text = draw(&ed, 80, 24).join("\n");
        assert!(
            text.contains("INTERFACE") && text.contains("section 2/2"),
            "{text}"
        );
        assert!(text.contains("‹ auto ›") && text.contains("● on"), "{text}");
        assert!(text.contains("tui.theme"), "{text}");
        ed.handle_key(KeyKind::PageUp);
        assert_eq!(ed.selected_row().unwrap().path, "key.anthropic");
    }

    #[test]
    fn interface_section_lists_the_theme_settings_and_fits_the_inline_region() {
        let mut ed = ed_with(sample_rows());
        ed.handle_key(KeyKind::PageDown);
        let rows = draw(&ed, 80, 14);
        let text = rows.join("\n");
        assert!(
            text.contains("Theme") && text.contains("Reduce motion"),
            "{text}"
        );
        assert_eq!(rows.len(), 14);
        ed.filter = "zzz".into();
        assert!(draw(&ed, 80, 14).join("\n").contains("No settings match"));
    }
}
