//! What this device has to offer, per category. Reads only; nothing here mutates local state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::files::{bundle, modified_ms, plain_name};
use super::{Body, Category, LocalItem, Skip};

pub(super) const TOOLS: [(&str, &str, &str); 4] = [
    (
        "claude",
        "Claude Code",
        "npm install -g @anthropic-ai/claude-code",
    ),
    ("codex", "Codex CLI", "npm install -g @openai/codex"),
    ("agy", "Antigravity", "see https://antigravity.google"),
    ("ollama", "Ollama", "https://ollama.com/download"),
];

pub(super) const OAUTH_LOGINS: [(&str, &str, &str); 2] = [
    ("codex", "Codex (ChatGPT)", "forge auth codex-oauth"),
    ("xai", "xAI (Grok)", "forge auth xai-oauth"),
];

pub(super) fn config_dir() -> Option<PathBuf> {
    forge_config::config_dir()
}

pub(super) fn on_path(command: &str) -> bool {
    let path = Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let candidate = dir.join(command);
            candidate.is_file() || cfg!(windows) && dir.join(format!("{command}.exe")).is_file()
        })
    })
}

fn titled(slot: &str) -> String {
    let mut chars = slot.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Every secret slot that may travel: provider keys, search keys, the long-lived Claude token and
/// the static secrets of configured MCP servers. Rotating OAuth slots are never listed.
pub(super) fn secret_slots() -> Vec<(String, String)> {
    let mut slots: Vec<(String, String)> = forge_config::known_key_providers()
        .map(|p| (p.to_string(), format!("{} key", titled(p))))
        .collect();
    slots.push(("brave".into(), "Brave Search key".into()));
    slots.push((
        "artificialanalysis".into(),
        "Artificial Analysis key".into(),
    ));
    slots.push((
        forge_config::CLAUDE_OAUTH_TOKEN_SLOT.into(),
        "Claude Code token".into(),
    ));
    for server in user_mcp().servers {
        for slot in server.keyring_keys() {
            slots.push((slot.clone(), format!("MCP {} secret ({slot})", server.name)));
        }
    }
    slots.retain(|(slot, _)| slot_is_syncable(slot));
    slots.sort();
    slots.dedup_by(|a, b| a.0 == b.0);
    slots
}

/// The only slot shapes a remote device may write. Blocks OAuth slots and Forge's own state.
pub(super) fn slot_is_syncable(slot: &str) -> bool {
    if slot.is_empty()
        || slot.len() > 160
        || slot.starts_with("provider-oauth")
        || slot.starts_with("mcp-oauth")
        || slot.starts_with("anywhere")
    {
        return false;
    }
    let known = forge_config::known_key_providers().any(|p| p == slot)
        || matches!(slot, "brave" | "artificialanalysis")
        || slot == forge_config::CLAUDE_OAUTH_TOKEN_SLOT;
    let mcp = slot.strip_prefix("mcp:").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    });
    known || mcp
}

fn user_mcp() -> forge_config::mcp::McpConfig {
    config_dir().map_or_else(Default::default, |dir| {
        forge_config::load_mcp_toml(&dir.join("mcp.toml"))
    })
}

pub(super) fn collect(category: Category) -> (Vec<LocalItem>, Vec<Skip>) {
    let mut skips = Vec::new();
    let items = match category {
        Category::Keys => keys(),
        Category::Skills => skills(&mut skips),
        Category::Settings => settings(&mut skips),
        Category::Mcp => mcp(&mut skips),
        Category::Instructions => instructions(&mut skips),
    };
    (items, skips)
}

fn item(
    category: Category,
    name: String,
    label: String,
    kind: &'static str,
    body: Body,
    modified_ms: Option<u64>,
) -> LocalItem {
    LocalItem {
        category,
        item: name,
        label,
        kind,
        body,
        modified_ms,
    }
}

fn keys() -> Vec<LocalItem> {
    let mut out = Vec::new();
    for (slot, label) in secret_slots() {
        if let Some(value) = forge_config::load_secret(&slot).filter(|v| !v.trim().is_empty()) {
            out.push(item(
                Category::Keys,
                slot,
                label,
                "user_setting",
                Body::Secret(value),
                None,
            ));
        }
    }
    for (provider, label, _) in OAUTH_LOGINS {
        if forge_config::provider_oauth::load_provider_oauth_tokens(provider).is_some() {
            out.push(marker(
                Category::Keys,
                format!("login/{provider}"),
                format!("{label} sign-in"),
            ));
        }
    }
    for server in user_mcp().servers {
        if forge_config::oauth::load_oauth_tokens(&server.name).is_some() {
            out.push(marker(
                Category::Keys,
                format!("login/mcp:{}", server.name),
                format!("MCP {} sign-in", server.name),
            ));
        }
    }
    out
}

/// A presence-only item: it carries no secret, only the fact that something is in use here.
fn marker(category: Category, name: String, label: String) -> LocalItem {
    item(
        category,
        name,
        label,
        "user_setting",
        Body::Plain(json!({ "present": true })),
        None,
    )
}

fn skills(skips: &mut Vec<Skip>) -> Vec<LocalItem> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let installed = installed_packs(&dir);
    let pack_files: Vec<&String> = installed.values().flat_map(|e| e.files.iter()).collect();
    for (sub, kind) in [
        ("skills", "skill"),
        ("commands", "command"),
        ("agents", "agent"),
    ] {
        let Ok(entries) = std::fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !plain_name(&name) || name.ends_with(".conflict") {
                continue;
            }
            if sub == "skills" && pack_files.iter().any(|f| **f == name) {
                continue;
            }
            let path = entry.path();
            let label = format!("{kind}: {name}");
            match bundle(&path) {
                Ok(Some(value)) => out.push(item(
                    Category::Skills,
                    format!("{kind}/{name}"),
                    label,
                    kind,
                    Body::Plain(value),
                    modified_ms(&path),
                )),
                Ok(None) => skips.push(Skip {
                    label,
                    reason: "larger than 4 MB".into(),
                }),
                Err(_) => {}
            }
        }
    }
    for (name, entry) in &installed {
        out.push(item(
            Category::Skills,
            format!("pack/{name}"),
            format!("skill pack: {name}"),
            "user_setting",
            Body::Plain(entry.value.clone()),
            None,
        ));
    }
    for (name, entry) in toml_table(&dir.join("marketplaces.toml"), "marketplaces") {
        out.push(item(
            Category::Skills,
            format!("marketplace/{name}"),
            format!("marketplace: {name}"),
            "user_setting",
            Body::Plain(entry),
            None,
        ));
    }
    out
}

struct Pack {
    files: Vec<String>,
}

impl Pack {
    fn from(value: &Value) -> Self {
        let files = value
            .get("files")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default();
        Self { files }
    }
}

fn installed_packs(dir: &Path) -> BTreeMap<String, PackEntryValue> {
    toml_table(&dir.join("installed-skills.toml"), "skills")
        .into_iter()
        .map(|(name, value)| {
            (
                name,
                PackEntryValue {
                    files: Pack::from(&value).files,
                    value,
                },
            )
        })
        .collect()
}

/// An installed pack's lockfile entry plus the directory names it owns.
pub(super) struct PackEntryValue {
    pub files: Vec<String>,
    pub value: Value,
}

fn toml_table(path: &Path, key: &str) -> Vec<(String, Value)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(root) = toml::from_str::<toml::Value>(&text) else {
        return Vec::new();
    };
    root.get(key)
        .and_then(toml::Value::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(name, v)| serde_json::to_value(v).ok().map(|v| (name.clone(), v)))
                .collect()
        })
        .unwrap_or_default()
}

fn settings(skips: &mut Vec<Skip>) -> Vec<LocalItem> {
    let mut out = Vec::new();
    let modified = config_dir().and_then(|d| modified_ms(&d.join("config.toml")));
    match forge_config::account_sync::read_portable_settings() {
        Ok((values, blocked)) => {
            for (key, value) in values {
                out.push(item(
                    Category::Settings,
                    format!("setting/{key}"),
                    format!("setting: {key}"),
                    "user_setting",
                    Body::Plain(value),
                    modified,
                ));
            }
            for key in blocked {
                skips.push(Skip {
                    label: format!("setting: {key}"),
                    reason: "holds a shell command, kept on this machine".into(),
                });
            }
        }
        Err(_) => skips.push(Skip {
            label: "config.toml".into(),
            reason: "could not be read".into(),
        }),
    }
    if let Some(path) = forge_config::keybindings_path().filter(|p| p.is_file()) {
        if let Ok(Some(value)) = bundle(&path) {
            out.push(item(
                Category::Settings,
                "file/keybindings.json".into(),
                "keybindings".into(),
                "user_setting",
                Body::Plain(value),
                modified_ms(&path),
            ));
        }
    }
    for (bin, label, _) in TOOLS {
        if on_path(bin) {
            out.push(marker(
                Category::Settings,
                format!("tool/{bin}"),
                format!("tool: {label}"),
            ));
        }
    }
    out
}

fn mcp(skips: &mut Vec<Skip>) -> Vec<LocalItem> {
    let Some(dir) = config_dir() else {
        return Vec::new();
    };
    let path = dir.join("mcp.toml");
    let modified = modified_ms(&path);
    let mut out = Vec::new();
    for server in user_mcp().servers {
        let label = format!("mcp: {}", server.name);
        if let Some(reason) = super::inline_secret::embedded_secret(&server) {
            skips.push(Skip { label, reason });
            continue;
        }
        if let Ok(value) = serde_json::to_value(&server) {
            out.push(item(
                Category::Mcp,
                format!("server/{}", server.name),
                label,
                "user_setting",
                Body::Plain(value),
                modified,
            ));
        }
    }
    out
}

fn instructions(skips: &mut Vec<Skip>) -> Vec<LocalItem> {
    let Some(path) = config_dir()
        .map(|d| d.join("AGENTS.md"))
        .filter(|p| p.is_file())
    else {
        return Vec::new();
    };
    match bundle(&path) {
        Ok(Some(value)) => vec![item(
            Category::Instructions,
            "file/AGENTS.md".into(),
            "instructions: AGENTS.md".into(),
            "user_setting",
            Body::Plain(value),
            modified_ms(&path),
        )],
        Ok(None) => {
            skips.push(Skip {
                label: "instructions: AGENTS.md".into(),
                reason: "larger than 4 MB".into(),
            });
            Vec::new()
        }
        Err(_) => Vec::new(),
    }
}
