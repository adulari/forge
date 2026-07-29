//! Dynamic configuration-editor descriptors and scoped mutation.

use super::*;

// Dynamic config editor backing (`/config`). The settable surface is *discovered* by walking the
// serialized Config — every scalar field appears automatically, and a newly-added field needs no
// extra code here. Complex sections (lists/maps: hooks, mcp, permission rules) are excluded; they
// have dedicated commands (`/hooks`, `/mcp`, …).
// ----------------------------------------------------------------------------------------------

/// Where a `/config` edit is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScope {
    /// `~/.config/forge/config.toml` — applies everywhere.
    User,
    /// `./.forge/config.toml` — repo-local override.
    Project,
}

/// A single editable scalar setting: its dotted path and current value/type.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingLeaf {
    pub path: String,
    pub value: SettingValue,
}

/// The typed value of a [`SettingLeaf`] (only scalars are editable here).
#[derive(Debug, Clone, PartialEq)]
pub enum SettingValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// A string list, edited as JSON so entries may safely contain commas or newlines.
    List(Vec<String>),
    /// A structured non-secret config value, edited as JSON.
    Json(serde_json::Value),
    /// An unset optional (serialized `null`) — edited as text, empty clears it.
    Unset,
}

impl SettingValue {
    /// A short type tag for the editor UI.
    pub fn type_tag(&self) -> &'static str {
        match self {
            SettingValue::Bool(_) => "bool",
            SettingValue::Int(_) => "int",
            SettingValue::Float(_) => "float",
            SettingValue::Str(_) | SettingValue::Unset => "text",
            SettingValue::List(_) => "list",
            SettingValue::Json(_) => "json",
        }
    }

    /// How the current value renders in the editor.
    pub fn display(&self) -> String {
        match self {
            SettingValue::Bool(b) => b.to_string(),
            SettingValue::Int(i) => i.to_string(),
            SettingValue::Float(f) => f.to_string(),
            SettingValue::Str(s) => s.clone(),
            SettingValue::List(values) => serde_json::to_string(values).unwrap_or_default(),
            SettingValue::Json(value) => serde_json::to_string_pretty(value).unwrap_or_default(),
            SettingValue::Unset => String::new(),
        }
    }
}

/// Top-level sections that are NOT scalar-editable here — each has its own command/flow.
const COMPLEX_SECTIONS: &[&str] = &[
    "hooks",
    "mcp",
    "permissions",
    "statusline",
    "keybinds",
    "providers",
];

/// The complex (table/array) config sections the flat `/config` editor can't surface as scalars.
/// They're listed read-only there with an "edit in $EDITOR" jump so they're at least discoverable.
pub fn complex_sections() -> &'static [&'static str] {
    COMPLEX_SECTIONS
}

/// One-line description of a complex section, for its read-only `/config` row.
pub fn complex_section_help(section: &str) -> &'static str {
    match section {
        "hooks" => "pre/post tool-use shell hooks — structured TOML, edit in $EDITOR",
        "mcp" => "external MCP servers — edit in $EDITOR (or .mcp.json / .forge/mcp.toml)",
        "permissions" => "allow/deny tool rules — structured TOML, edit in $EDITOR",
        _ => "structured section — edit in $EDITOR",
    }
}

/// Importance order for the editor: these path prefixes sort first (in this order); everything else
/// follows alphabetically. New fields therefore appear automatically, just lower in the list until
/// curated here.
const PRIORITY_PREFIXES: &[&str] = &[
    "permission_mode",
    "mesh.credit_mode",
    "mesh.daily_budget_usd",
    "mesh.monthly_cap_usd",
    "mesh.weekly_budget_usd",
    "local.autostart",
    "local.model",
    "tui.fullscreen",
    "tui.mouse_capture",
    "project.auto_initialize",
    "recap.enabled",
    "mesh",
    "local",
    "tui",
];

/// Discover every scalar setting from the *effective* config (defaults + user + project), as
/// importance-ordered dotted-path leaves. Arrays and the complex sections are skipped.
pub fn config_leaves() -> Vec<SettingLeaf> {
    let cfg = load().unwrap_or_default();
    let value = serde_json::to_value(&cfg).unwrap_or(serde_json::Value::Null);
    let mut out = Vec::new();
    flatten_value("", &value, &mut out);
    out.sort_by(|a, b| {
        priority_rank(&a.path)
            .cmp(&priority_rank(&b.path))
            .then_with(|| a.path.cmp(&b.path))
    });
    out
}

fn priority_rank(path: &str) -> usize {
    // Most specific matching prefix wins (so `mesh.credit_mode` beats the `mesh` catch-all).
    PRIORITY_PREFIXES
        .iter()
        .enumerate()
        .filter(|(_, p)| path == **p || path.starts_with(&format!("{p}.")))
        .map(|(i, _)| i)
        .min()
        .unwrap_or(usize::MAX)
}

/// The editing control a setting should use in the `/config` UI.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingKind {
    /// On/off — toggled, never typed.
    Bool,
    Int,
    Float,
    /// A JSON array of strings, edited with an item-list control.
    List,
    /// A structured non-secret value, edited as JSON.
    Json,
    /// A fixed set of valid values — cycled / picked, never typed.
    Enum(Vec<&'static str>),
    Text,
}

/// A fully-described setting for the friendly `/config` editor: friendly label + group, help, the
/// editing control, the current/default values, whether it's been overridden, and from where.
#[derive(Debug, Clone)]
pub struct SettingDescriptor {
    pub path: String,
    /// Section header it groups under (e.g. "Mesh & Cost").
    pub group: String,
    /// Friendly display name (e.g. "Daily spend cap").
    pub label: String,
    pub help: Option<String>,
    pub kind: SettingKind,
    /// Current effective value.
    pub value: SettingValue,
    /// Built-in default value.
    pub default: SettingValue,
    /// True when set in a config file (overrides the default).
    pub modified: bool,
    /// Where the effective value comes from: "project" | "user" | "default".
    pub source: &'static str,
}

/// Valid values for an enum-typed setting (so the editor can cycle them instead of free text).
pub fn setting_options(path: &str) -> Option<Vec<&'static str>> {
    Some(match path {
        "permission_mode" => vec!["default", "accept-edits", "bypass", "plan"],
        "mesh.credit_mode" => vec!["normal", "frugal", "strict"],
        "mesh.classifier" => vec!["heuristic", "llm"],
        "mesh.default_effort" => vec!["low", "medium", "high", "xhigh", "max"],
        "lattice.embeddings.backend" => vec!["auto", "ollama", "openai", "gemini"],
        _ => return None,
    })
}

/// The section group and friendly label for a setting path. Curated for common settings; anything
/// else derives a sensible group (from the first path segment) + label (humanized last segment), so
/// new fields still slot in nicely without code changes.
pub fn setting_group_and_label(path: &str) -> (String, String) {
    let curated = match path {
        "permission_mode" => Some(("Safety", "Permission mode")),
        "mesh.credit_mode" => Some(("Mesh & Cost", "Credit conservation")),
        "mesh.daily_budget_usd" => Some(("Mesh & Cost", "Daily spend cap (USD)")),
        "mesh.weekly_budget_usd" => Some(("Mesh & Cost", "Weekly spend cap (USD)")),
        "mesh.monthly_cap_usd" => Some(("Mesh & Cost", "Monthly spend cap (USD)")),
        "mesh.classifier" => Some(("Mesh & Cost", "Task classifier")),
        "mesh.classifier_model" => Some(("Mesh & Cost", "Classifier model")),
        "mesh.classifier_activity_focused" => {
            Some(("Mesh & Cost", "Focus trailing prompt activity"))
        }
        "mesh.bridge_mcp_external" => Some(("Mesh & Cost", "Bridge external MCP")),
        "mesh.prefer_subscription" => Some(("Mesh & Cost", "Prefer subscriptions")),
        "mesh.max_output_tokens" => Some(("Mesh & Cost", "Max output tokens")),
        "mesh.architect_mode" => Some(("Mesh & Cost", "Architect mode")),
        "mesh.architect_model" => Some(("Mesh & Cost", "Architect model")),
        "mesh.editor_model" => Some(("Mesh & Cost", "Editor model")),
        "mesh.self_review" => Some(("Mesh & Cost", "Self-review writes")),
        "mesh.default_effort" => Some(("Mesh & Cost", "Default reasoning effort")),
        "local.autostart" => Some(("Local LLM", "Auto-start on launch")),
        "local.model" => Some(("Local LLM", "Model (Ollama tag)")),
        "local.endpoint" => Some(("Local LLM", "Ollama endpoint")),
        "tui.fullscreen" => Some(("Interface", "Full-screen TUI")),
        "tui.mouse_capture" => Some(("Interface", "Mouse wheel scroll")),
        "project.auto_initialize" => Some(("Project", "Auto-initialize projects")),
        "recap.enabled" => Some(("Interface", "Per-turn recap")),
        "update.check" => Some(("Interface", "Check for updates")),
        "shell.explain_errors" => Some(("Shell", "Explain failed commands")),
        "lattice.enabled" => Some(("Code Intelligence", "Enabled")),
        "lattice.inject" => Some(("Code Intelligence", "Auto-inject context")),
        "lattice.watch" => Some(("Code Intelligence", "Watch & reindex")),
        "lattice.embeddings.backend" => Some(("Code Intelligence", "Embeddings backend")),
        "autofix.enabled" => Some(("Autofix", "Enabled")),
        "autofix.max_iterations" => Some(("Autofix", "Max iterations")),
        "autofix.auto_detect" => Some(("Autofix", "Auto-detect commands")),
        "assay.gate_enabled" => Some(("Assay", "Review gate")),
        "assay.max_cost_usd" => Some(("Assay", "Max cost (USD)")),
        "git.coauthor" => Some(("Git", "Co-author commits")),
        "lsp.enabled" => Some(("Code Intelligence", "LSP diagnostics")),
        "mesh.auto_orchestrate" => Some(("Behaviour", "Auto-orchestrate")),
        _ => None,
    };
    if let Some((g, l)) = curated {
        return (g.to_string(), l.to_string());
    }
    // Fallback: group from the top segment, label humanized from the last segment.
    let top = path.split('.').next().unwrap_or(path);
    let last = path.rsplit('.').next().unwrap_or(path);
    (humanize(top), humanize(last))
}

fn humanize(s: &str) -> String {
    let mut out = String::new();
    for (i, word) in s.split('_').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut cs = word.chars();
        if let Some(c) = cs.next() {
            out.extend(c.to_uppercase());
            out.push_str(cs.as_str());
        }
    }
    out
}

/// Build the full descriptor list for the friendly `/config` editor: every scalar setting with its
/// group, label, help, control kind, value, default, modified flag, and source — importance-ordered.
pub fn config_descriptors() -> Vec<SettingDescriptor> {
    // Effective leaves (defaults + user + project).
    let leaves = config_leaves();
    // Default-only values, for the "default" column + modified detection.
    let default_value = serde_json::to_value(Config::default()).unwrap_or(serde_json::Value::Null);
    let mut default_leaves = Vec::new();
    flatten_value("", &default_value, &mut default_leaves);
    let defaults: std::collections::HashMap<String, SettingValue> = default_leaves
        .into_iter()
        .map(|l| (l.path, l.value))
        .collect();
    // Which file set each path (for source + modified).
    let user_table = read_table(scope_path(ConfigScope::User).ok().as_deref());
    let project_table = read_table(Some(std::path::Path::new("./.forge/config.toml")));

    let mut descriptors: Vec<SettingDescriptor> = leaves
        .into_iter()
        .map(|l| {
            let (group, label) = setting_group_and_label(&l.path);
            let kind = match setting_options(&l.path) {
                Some(opts) => SettingKind::Enum(opts),
                None => match l.value {
                    SettingValue::Bool(_) => SettingKind::Bool,
                    SettingValue::Int(_) => SettingKind::Int,
                    SettingValue::Float(_) => SettingKind::Float,
                    SettingValue::List(_) => SettingKind::List,
                    SettingValue::Json(_) => SettingKind::Json,
                    _ => SettingKind::Text,
                },
            };
            let in_project = project_table
                .as_ref()
                .is_some_and(|t| dotted_present(t, &l.path));
            let in_user = user_table
                .as_ref()
                .is_some_and(|t| dotted_present(t, &l.path));
            let source = if in_project {
                "project"
            } else if in_user {
                "user"
            } else {
                "default"
            };
            let default = defaults
                .get(&l.path)
                .cloned()
                .unwrap_or(SettingValue::Unset);
            SettingDescriptor {
                help: setting_help(&l.path).map(str::to_string),
                kind,
                value: l.value,
                modified: in_project || in_user,
                default,
                group,
                label,
                source,
                path: l.path,
            }
        })
        .collect();
    // Group rows so each section is contiguous; sections ordered by the importance of their first
    // member (descriptors are already importance-ordered), rows kept in that order within a group.
    let mut group_order: Vec<String> = Vec::new();
    for d in &descriptors {
        if !group_order.contains(&d.group) {
            group_order.push(d.group.clone());
        }
    }
    descriptors.sort_by_key(|d| {
        group_order
            .iter()
            .position(|g| g == &d.group)
            .unwrap_or(usize::MAX)
    });
    descriptors
}

fn read_table(path: Option<&std::path::Path>) -> Option<toml::Table> {
    let p = path?;
    std::fs::read_to_string(p).ok()?.parse().ok()
}

fn dotted_present(table: &toml::Table, path: &str) -> bool {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = table;
    for p in &parts[..parts.len() - 1] {
        match cur.get(*p).and_then(|v| v.as_table()) {
            Some(t) => cur = t,
            None => return false,
        }
    }
    cur.contains_key(parts[parts.len() - 1])
}

fn flatten_value(prefix: &str, value: &serde_json::Value, out: &mut Vec<SettingLeaf>) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                // Skip complex top-level sections entirely (their own commands own them).
                if prefix.is_empty() && COMPLEX_SECTIONS.contains(&k.as_str()) {
                    if k == "permissions"
                        || k == "statusline"
                        || k == "hooks"
                        || k == "keybinds"
                        || k == "providers"
                    {
                        out.push(leaf(k, SettingValue::Json(v.clone())));
                    }
                    continue;
                }
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten_value(&path, v, out);
            }
        }
        // Arrays of strings are a first-class editable list; structured arrays stay out of the
        // scalar editor so secret-bearing/complex sections retain dedicated safe flows.
        Value::Array(items) if items.iter().all(|item| item.is_string()) => out.push(leaf(
            prefix,
            SettingValue::List(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
            ),
        )),
        Value::Array(_) => {}
        Value::Bool(b) => out.push(leaf(prefix, SettingValue::Bool(*b))),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                out.push(leaf(prefix, SettingValue::Int(i)));
            } else if let Some(f) = n.as_f64() {
                out.push(leaf(prefix, SettingValue::Float(f)));
            }
        }
        Value::String(s) => out.push(leaf(prefix, SettingValue::Str(s.clone()))),
        Value::Null => out.push(leaf(prefix, SettingValue::Unset)),
    }
}

fn leaf(path: &str, value: SettingValue) -> SettingLeaf {
    SettingLeaf {
        path: path.to_string(),
        value,
    }
}

/// One-line help for a setting path, shown in the `/config` editor. `None` for paths without a
/// curated description (they still appear and are editable — just without help text).
pub fn setting_help(path: &str) -> Option<&'static str> {
    Some(match path {
        "permission_mode" => "Tool-safety posture for new sessions: default · accept-edits · bypass · plan.",
        "mesh.credit_mode" => "Subscription conservation: normal · frugal · strict (spread work off paid plans).",
        "mesh.daily_budget_usd" => "Hard daily spend cap (USD) across sessions; the mesh downshifts/stops near it.",
        "mesh.weekly_budget_usd" => "Hard weekly spend cap (USD). Empty = unlimited.",
        "mesh.monthly_cap_usd" => "Hard monthly spend cap (USD). Empty = unlimited.",
        "mesh.classifier" => "Task-tier classifier: heuristic (instant, no call) or llm (a cheap model labels each turn).",
        "mesh.classifier_model" => "Fixed model the llm classifier calls. Default: groq::llama-3.3-70b-versatile. Set an available capable model to override it.",
        "mesh.classifier_activity_focused" => "Compatibility mode for unstructured prompts: classify only the final non-empty paragraph. Prefer API system/user roles or `forge run --system`.",
        "mesh.bridge_mcp_external" => "Connect external project MCP servers (dual-graph/helm/…) inside the CLI bridge. On by default; servers connect concurrently in the background with a timeout, so slow servers are skipped instead of stalling turns. Set false to disable. Forge core tools stay either way.",
        "mesh.prefer_subscription" => "Prefer $0 CLI-bridge subscriptions over a metered API model on a tie.",
        "mesh.max_output_tokens" => {
            "Explicit cap on tokens a model may generate per call (0 = provider/model maximum)."
        }
        "mesh.architect_mode" => "Use a stronger 'architect' model to plan, a cheaper one to edit.",
        "mesh.architect_model" => "Model used for the architect/planning pass when architect_mode is on.",
        "mesh.editor_model" => "Model used to apply edits when architect_mode is on.",
        "mesh.self_review" => "After a write turn, have the model review its own diff before finishing.",
        "mesh.default_effort" => "Default reasoning effort for models that support it (low/medium/high/…).",
        "local.autostart" => "Start the local Ollama model automatically when `forge chat` launches.",
        "local.model" => "Ollama tag to auto-start (e.g. gemma4:12b). Set it via `forge local install`.",
        "local.endpoint" => "Ollama HTTP endpoint (default http://localhost:11434).",
        "tui.fullscreen" => "Full-screen TUI on the alternate screen. Off = inline in native scrollback.",
        "tui.mouse_capture" => "Wheel scrolls the transcript in full-screen mode (minimal button+wheel reporting, no motion tracking — native click-drag text selection still works). Default on. Off disables mouse reporting entirely; scroll with PgUp/PgDn/Home/End.",
        "project.auto_initialize" => "Run a model-backed, tailored Forge setup once when opening an uninitialized project.",
        "recap.enabled" => "Show a one-line AI recap after each completed turn.",
        "update.check" => "Check GitHub for a newer Forge release on startup (throttled to once a day).",
        "shell.explain_errors" => "When a shell command fails, the AI explains the likely cause + a fix.",
        "lattice.enabled" => "Build/maintain the code-intelligence graph (`forge lattice`).",
        "lattice.inject" => "Auto-inject relevant code into each turn before the model call.",
        "lattice.watch" => "Reindex changed files automatically as you edit.",
        "autofix.enabled" => "After edits, run lint/test and feed failures back so the model self-heals.",
        "autofix.max_iterations" => "Max self-heal passes before giving up.",
        "autofix.auto_detect" => "Detect lint/test commands from project structure when lint_cmd/test_cmd are empty (Cargo.toml → cargo check; package.json → npm run lint).",
        "assay.gate_enabled" => "Run an Assay review on write turns before they finish.",
        "assay.max_cost_usd" => "Per-run cost ceiling for the Assay critic crew.",
        "git.coauthor" => "Install a commit hook stamping Co-Authored-By: Forge and stripping CLI co-authors.",
        "lsp.enabled" => "Feed language-server diagnostics back into the turn after edits.",
        "mesh.auto_orchestrate" => "Inject the orchestration framework every session: skills first, highest-level tool, subagents/MCP/web/Lattice — no need to /orchestrate manually.",
        _ => return None,
    })
}

/// The config file path for a scope.
pub fn scope_path(scope: ConfigScope) -> Result<PathBuf, ConfigError> {
    match scope {
        ConfigScope::User => Ok(config_dir()
            .ok_or(ConfigError::NoConfigDir)?
            .join("config.toml")),
        ConfigScope::Project => Ok(PathBuf::from("./.forge/config.toml")),
    }
}

/// Set a dotted-path scalar in the given scope's `config.toml`, preserving every other key. `raw` is
/// coerced to the leaf's existing type (bool/int/float/text); an empty value on an optional clears
/// it. The result is validated by re-extracting the whole `Config` — a bad value (e.g. an invalid
/// enum) is rejected and the file is left untouched.
pub fn set_config_value(scope: ConfigScope, path: &str, raw: &str) -> Result<(), ConfigError> {
    let leaves = config_leaves();
    let existing = leaves.iter().find(|l| l.path == path);
    let coerced = coerce_value(raw, existing.map(|l| &l.value))?;

    let file = scope_path(scope)?;
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write(e.to_string()))?;
    }
    let mut root: toml::Table = std::fs::read_to_string(&file)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    match coerced {
        Some(v) => set_dotted(&mut root, path, v),
        None => remove_dotted(&mut root, path), // empty → clear the optional
    }

    // Validate: the file must still extract to a Config layered over the defaults.
    let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(&body))
        .extract::<Config>()
        .map_err(|e| ConfigError::Write(format!("invalid value for {path}: {e}")))?;

    std::fs::write(&file, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(())
}

/// Reset a setting to its default by removing it from the given scope's `config.toml` (and, when
/// resetting user scope, also from the project file if present, so the default actually takes
/// effect). No-op if absent. The remaining file is validated.
pub fn reset_config_value(scope: ConfigScope, path: &str) -> Result<(), ConfigError> {
    let file = scope_path(scope)?;
    let Some(text) = std::fs::read_to_string(&file).ok() else {
        return Ok(()); // nothing written → already default
    };
    let mut root: toml::Table = text.parse().unwrap_or_default();
    remove_dotted(&mut root, path);
    let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::string(&body))
        .extract::<Config>()
        .map_err(|e| ConfigError::Write(format!("invalid config after reset of {path}: {e}")))?;
    std::fs::write(&file, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(())
}

/// Coerce raw input to a TOML value matching the existing leaf's type. `None` = clear (empty input
/// on an optional/text). Errors on a malformed bool/number.
fn coerce_value(
    raw: &str,
    existing: Option<&SettingValue>,
) -> Result<Option<toml::Value>, ConfigError> {
    let t = raw.trim();
    match existing {
        Some(SettingValue::Bool(_)) => {
            let b = match t.to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" => true,
                "false" | "off" | "no" | "0" => false,
                _ => {
                    return Err(ConfigError::Write(format!(
                        "expected a boolean, got '{raw}'"
                    )))
                }
            };
            Ok(Some(toml::Value::Boolean(b)))
        }
        Some(SettingValue::Int(_)) => t
            .parse::<i64>()
            .map(|i| Some(toml::Value::Integer(i)))
            .map_err(|_| ConfigError::Write(format!("expected an integer, got '{raw}'"))),
        Some(SettingValue::Float(_)) => t
            .parse::<f64>()
            .map(|f| Some(toml::Value::Float(f)))
            .map_err(|_| ConfigError::Write(format!("expected a number, got '{raw}'"))),
        Some(SettingValue::List(_)) => serde_json::from_str::<Vec<String>>(t)
            .map(|items| toml::Value::Array(items.into_iter().map(toml::Value::String).collect()))
            .map(Some)
            .map_err(|_| ConfigError::Write("expected a JSON string array".to_string())),
        Some(SettingValue::Json(_)) => serde_json::from_str::<serde_json::Value>(t)
            .map_err(|_| ConfigError::Write("expected valid JSON".to_string()))
            .and_then(|value| {
                toml::Value::try_from(value).map(Some).map_err(|_| {
                    ConfigError::Write("JSON value cannot be saved to TOML".to_string())
                })
            }),
        // Text or unset/optional: empty clears, otherwise a string.
        _ => {
            if t.is_empty() {
                Ok(None)
            } else {
                Ok(Some(toml::Value::String(t.to_string())))
            }
        }
    }
}

fn set_dotted(root: &mut toml::Table, path: &str, val: toml::Value) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for p in &parts[..parts.len() - 1] {
        let entry = cur
            .entry(p.to_string())
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(toml::Table::new());
        }
        cur = entry.as_table_mut().unwrap();
    }
    cur.insert(parts[parts.len() - 1].to_string(), val);
}

fn remove_dotted(root: &mut toml::Table, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    let mut cur = root;
    for p in &parts[..parts.len() - 1] {
        match cur.get_mut(*p).and_then(|v| v.as_table_mut()) {
            Some(t) => cur = t,
            None => return,
        }
    }
    cur.remove(parts[parts.len() - 1]);
}

/// Persist the CLI-bridge subscription plans into the user `config.toml`, preserving every other
/// key already in the file (`forge init`). Returns the path written. Set `[mesh.subscriptions]`
/// without disturbing the rest of the config — secrets are NEVER written here (keys go to the
/// keyring; ADR-0007).
pub fn write_subscriptions(subs: &HashMap<String, String>) -> Result<PathBuf, ConfigError> {
    let dir = config_dir().ok_or(ConfigError::NoConfigDir)?;
    std::fs::create_dir_all(&dir).map_err(|e| ConfigError::Write(e.to_string()))?;
    let path = dir.join("config.toml");
    write_subscriptions_at(&path, subs)?;
    Ok(path)
}

/// The file half of [`write_subscriptions`] against an explicit path: set `[mesh.subscriptions]`
/// in the TOML at `path`, preserving every other key. Split out so it can be tested without
/// touching the real per-user config directory.
fn write_subscriptions_at(
    path: &std::path::Path,
    subs: &HashMap<String, String>,
) -> Result<(), ConfigError> {
    let mut root: toml::Table = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_default();

    let mesh = root
        .entry("mesh".to_string())
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    if !mesh.is_table() {
        *mesh = toml::Value::Table(toml::Table::new());
    }
    if let toml::Value::Table(mesh_t) = mesh {
        let sub_t: toml::Table = subs
            .iter()
            .map(|(k, v)| (k.clone(), toml::Value::String(v.clone())))
            .collect();
        mesh_t.insert("subscriptions".to_string(), toml::Value::Table(sub_t));
    }
    let body = toml::to_string_pretty(&root).map_err(|e| ConfigError::Write(e.to_string()))?;
    std::fs::write(path, body).map_err(|e| ConfigError::Write(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub(crate) fn priority_rank(path: &str) -> usize {
        super::priority_rank(path)
    }

    pub(crate) fn flatten_value(
        prefix: &str,
        value: &serde_json::Value,
        out: &mut Vec<SettingLeaf>,
    ) {
        super::flatten_value(prefix, value, out);
    }

    pub(crate) fn coerce_value(
        raw: &str,
        existing: Option<&SettingValue>,
    ) -> Result<Option<toml::Value>, ConfigError> {
        super::coerce_value(raw, existing)
    }

    pub(crate) fn write_subscriptions_at(
        path: &std::path::Path,
        subs: &HashMap<String, String>,
    ) -> Result<(), ConfigError> {
        super::write_subscriptions_at(path, subs)
    }

    pub(crate) fn set_dotted(root: &mut toml::Table, path: &str, value: toml::Value) {
        super::set_dotted(root, path, value);
    }

    pub(crate) fn remove_dotted(root: &mut toml::Table, path: &str) {
        super::remove_dotted(root, path);
    }
}
