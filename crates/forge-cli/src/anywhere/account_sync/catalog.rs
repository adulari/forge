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
    for server in user_mcp().0.servers {
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

fn user_mcp() -> (forge_config::mcp::McpConfig, Option<String>) {
    config_dir().map_or_else(Default::default, |dir| {
        forge_config::mcp::load_mcp_toml_with_diagnostic(&dir.join("mcp.toml"))
    })
}

fn unreadable(label: &str, reason: &str, guard: &str) -> Skip {
    Skip {
        label: label.into(),
        reason: reason.into(),
        guard: Some(guard.into()),
    }
}

pub(super) fn collect(category: Category) -> (Vec<LocalItem>, Vec<Skip>) {
    let mut skips = Vec::new();
    let items = match category {
        Category::Keys => keys(&mut skips),
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

fn keys(skips: &mut Vec<Skip>) -> Vec<LocalItem> {
    let mut out = Vec::new();
    let (mcp_config, mcp_problem) = user_mcp();
    if mcp_problem.is_some() {
        skips.push(unreadable(
            "mcp.toml",
            "could not be read, MCP secrets are left alone until it is fixed",
            "mcp:",
        ));
        skips.push(unreadable("mcp.toml", "could not be read", "login/mcp:"));
    }
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
    for server in mcp_config.servers {
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
    let installed = match installed_packs(&dir) {
        Ok(installed) => installed,
        Err(reason) => {
            skips.push(unreadable("installed-skills.toml", &reason, "pack/"));
            skips.push(unreadable("installed-skills.toml", &reason, "skill/"));
            BTreeMap::new()
        }
    };
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
                    guard: None,
                }),
                Err(_) => skips.push(unreadable(
                    &label,
                    "could not be read",
                    &format!("{kind}/{name}"),
                )),
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
    let marketplaces = toml_table(&dir.join("marketplaces.toml"), "marketplaces");
    if let Err(reason) = &marketplaces {
        skips.push(unreadable("marketplaces.toml", reason, "marketplace/"));
    }
    for (name, entry) in marketplaces.unwrap_or_default() {
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

fn installed_packs(dir: &Path) -> Result<BTreeMap<String, PackEntryValue>, String> {
    Ok(toml_table(&dir.join("installed-skills.toml"), "skills")?
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
        .collect())
}

/// An installed pack's lockfile entry plus the directory names it owns.
pub(super) struct PackEntryValue {
    pub files: Vec<String>,
    pub value: Value,
}

/// A missing file is an empty table; one that exists but cannot be read or parsed is an error, so
/// the caller never mistakes a typo for "the user removed everything".
fn toml_table(path: &Path, key: &str) -> Result<Vec<(String, Value)>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err("could not be read".into()),
    };
    let root = toml::from_str::<toml::Value>(&text).map_err(|_| "is not valid TOML".to_string())?;
    Ok(root
        .get(key)
        .and_then(toml::Value::as_table)
        .map(|t| {
            t.iter()
                .filter_map(|(name, v)| serde_json::to_value(v).ok().map(|v| (name.clone(), v)))
                .collect()
        })
        .unwrap_or_default())
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
                    guard: None,
                });
            }
        }
        Err(_) => skips.push(unreadable("config.toml", "could not be read", "setting/")),
    }
    if let Some(path) = forge_config::keybindings_path().filter(|p| p.is_file()) {
        match bundle(&path) {
            Ok(Some(value)) => out.push(item(
                Category::Settings,
                "file/keybindings.json".into(),
                "keybindings".into(),
                "user_setting",
                Body::Plain(value),
                modified_ms(&path),
            )),
            Ok(None) => {}
            Err(_) => skips.push(unreadable(
                "keybindings",
                "could not be read",
                "file/keybindings.json",
            )),
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
    let (config, problem) = user_mcp();
    if problem.is_some() {
        skips.push(unreadable(
            "mcp.toml",
            "is not valid TOML or could not be read",
            "server/",
        ));
    }
    for server in config.servers {
        let label = format!("mcp: {}", server.name);
        if let Some(reason) = embedded_secret(&server) {
            skips.push(Skip {
                label,
                reason,
                guard: None,
            });
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

/// A definition that carries a credential inline would leak it through a channel that is only
/// consented to for definitions, so it stays local.
fn embedded_secret(server: &forge_config::mcp::McpServerConfig) -> Option<String> {
    use forge_config::mcp::McpTransport;
    let reason = "holds a credential inline; move it to the keyring (`forge mcp`)".to_string();
    match &server.transport {
        McpTransport::Stdio { args, env, .. } => {
            let inline_env = env.iter().any(|(k, v)| {
                forge_config::looks_secret(k) && !v.is_empty() && !v.starts_with("${")
            });
            (inline_env || args.iter().any(|a| forge_config::embeds_credentials(a)))
                .then_some(reason)
        }
        McpTransport::Http { url, headers } | McpTransport::Sse { url, headers } => {
            let inline = headers.iter().any(|(k, v)| {
                (forge_config::looks_secret(k) || k.eq_ignore_ascii_case("authorization"))
                    && !v.starts_with("${")
            });
            (inline || forge_config::embeds_credentials(url)).then_some(reason)
        }
    }
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
                guard: None,
            });
            Vec::new()
        }
        Err(_) => {
            skips.push(unreadable(
                "instructions: AGENTS.md",
                "could not be read",
                "file/AGENTS.md",
            ));
            Vec::new()
        }
    }
}
