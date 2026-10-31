//! Multi-device reconcile scenarios on top of the shared in-process harness in `tests.rs`: every
//! device is a temp config directory plus its own store, and the sync feed is record copying.

use std::path::PathBuf;

use super::reconcile::run_cycle;
use forge_store::RemoteSyncRecord;

use super::super::sync_conflict;
use super::tests::{live_ids_for, machine, machine_with_epochs, transfer, write, Env, Machine};
use super::vault::Vault;

fn skills_only() -> forge_config::AccountSyncConfig {
    forge_config::AccountSyncConfig {
        skills: true,
        ..Default::default()
    }
}

async fn cycle(env: &Env, m: &Machine, config: &forge_config::AccountSyncConfig) {
    env.enter(m);
    m.store.set_sync_journal_enabled(true).unwrap();
    run_cycle(&m.store, &m.state, config).await.unwrap();
}

/// The sync service reduced to what reconcile depends on: an append-only feed in which the first
/// upload of a (kind, id, revision) is final and a different one for the same key is rejected.
#[derive(Default)]
struct Feed {
    records: Vec<RemoteSyncRecord>,
}

impl Feed {
    fn upload(&mut self, m: &Machine) {
        m.store.set_sync_journal_enabled(true).unwrap();
        let device = device_of(m);
        for entry in m.store.pending_sync_journal(1000).unwrap() {
            let hash = *entry.content_hash.as_slice().first_chunk::<32>().unwrap();
            let same_key = self.records.iter().find(|r| {
                r.record_kind == entry.record_kind
                    && r.stable_id == entry.stable_id
                    && r.revision == entry.revision
            });
            match same_key {
                Some(existing) if existing.content_hash == hash => {
                    m.store.mark_sync_journal_uploaded(&[entry.id], 1).unwrap();
                }
                Some(_) => {
                    assert!(sync_conflict::supersede(&m.store, device, &entry, 1).unwrap());
                }
                None => {
                    self.records.push(RemoteSyncRecord {
                        cursor: self.records.len() as i64 + 1,
                        sender_device_id: device,
                        record_kind: entry.record_kind.clone(),
                        stable_id: entry.stable_id.clone(),
                        operation: entry.operation.clone(),
                        revision: entry.revision,
                        logical_clock: entry.logical_clock,
                        base_hash: entry.base_hash,
                        content_hash: hash,
                        payload: entry.payload.clone(),
                    });
                    m.store.mark_sync_journal_uploaded(&[entry.id], 1).unwrap();
                }
            }
        }
    }

    fn download(&self, m: &Machine) {
        let device = device_of(m);
        let seen = m.store.sync_download_cursor().unwrap();
        for record in self.records.iter().filter(|r| r.cursor > seen) {
            if record.sender_device_id != device {
                m.store.stage_remote_sync_record(record).unwrap();
            }
        }
        m.store
            .advance_sync_download_cursor(self.records.len() as i64)
            .unwrap();
        m.store.apply_staged_portable_records(device, 1000).unwrap();
    }
}

fn device_of(m: &Machine) -> [u8; 16] {
    let mut id = [0u8; 16];
    id.copy_from_slice(&hex::decode(m.state.device_id.as_deref().unwrap()).unwrap());
    id
}

/// One pass of the daemon on every device: upload, download and apply, then reconcile.
async fn round(
    env: &Env,
    feed: &mut Feed,
    devices: &[&Machine],
    config: &forge_config::AccountSyncConfig,
) {
    for m in devices {
        feed.upload(m);
    }
    for m in devices {
        feed.download(m);
    }
    for m in devices {
        cycle(env, m, config).await;
    }
}

async fn settle(
    env: &Env,
    feed: &mut Feed,
    devices: &[&Machine],
    config: &forge_config::AccountSyncConfig,
) {
    for _ in 0..8 {
        round(env, feed, devices, config).await;
    }
}

fn read(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn clocks(m: &Machine) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for kind in ["user_setting", "skill", "command", "agent"] {
        for r in m.store.portable_sync_records(kind).unwrap() {
            out.push((r.stable_id, r.logical_clock));
        }
    }
    out.sort();
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn collapsing_duplicate_ids_keeps_the_newest_edit_on_the_device_that_made_it() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let config = skills_only();
    let epoch_sets: [&[(u32, u8)]; 2] = [&[(1, 0x44), (2, 0x55)], &[(2, 0x55)]];
    let ids: Vec<String> = epoch_sets
        .iter()
        .map(|e| {
            Vault::from_state(&machine_with_epochs(1, e).state)
                .unwrap()
                .stable_id("skills", "skill/foo")
        })
        .collect();
    // The device that owns the smaller id must be the lower sender, so the other device's record is
    // both the newest of the pair and not the canonical one.
    let (low_set, high_set) = if ids[0] < ids[1] {
        (epoch_sets[0], epoch_sets[1])
    } else {
        (epoch_sets[1], epoch_sets[0])
    };
    let a = machine_with_epochs(0xA1, low_set);
    let b = machine_with_epochs(0xB2, high_set);
    write(a.config.join("forge/skills/foo/SKILL.md"), "from a");
    write(b.config.join("forge/skills/foo/SKILL.md"), "from b");
    cycle(&env, &a, &config).await;
    cycle(&env, &b, &config).await;
    assert_ne!(live_ids_for(&a, "skill/foo"), live_ids_for(&b, "skill/foo"));

    settle(&env, &mut Feed::default(), &[&a, &b], &config).await;

    for m in [&a, &b] {
        assert_eq!(
            read(m.config.join("forge/skills/foo/SKILL.md")).as_deref(),
            Some("from b"),
            "the winning edit must be on every device, including the one that made it"
        );
        assert_eq!(live_ids_for(m, "skill/foo").len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_mcp_file_is_not_a_mass_delete() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let config = forge_config::AccountSyncConfig {
        mcp: true,
        skills: true,
        ..Default::default()
    };
    let a = machine(0xA1);
    let b = machine(0xB2);
    let good = "[[servers]]\nname = \"docs\"\n[servers.transport]\ntype = \"http\"\nurl = \"https://example.com/mcp\"\n";
    write(a.config.join("forge/mcp.toml"), good);
    write(
        a.config.join("forge/marketplaces.toml"),
        "[marketplaces.acme]\nurl = \"https://example.com/m\"\n",
    );
    cycle(&env, &a, &config).await;
    transfer(&a, &b);
    cycle(&env, &b, &config).await;
    assert!(read(b.config.join("forge/mcp.toml"))
        .unwrap()
        .contains("docs"));
    assert!(read(b.config.join("forge/marketplaces.toml"))
        .unwrap_or_else(|| panic!(
            "marketplaces.toml is missing: {:?}",
            b.store
                .account_sync_items(None)
                .unwrap()
                .iter()
                .map(|r| (&r.item, &r.status, &r.detail))
                .collect::<Vec<_>>()
        ))
        .contains("acme"));

    write(a.config.join("forge/mcp.toml"), "[[servers]\nname = oops");
    write(
        a.config.join("forge/marketplaces.toml"),
        "this is = = not toml",
    );
    cycle(&env, &a, &config).await;
    cycle(&env, &a, &config).await;
    assert!(
        clocks(&a).iter().all(|(_, c)| *c == 1),
        "a parse failure must not upload tombstones: {:?}",
        clocks(&a)
    );
    transfer(&a, &b);
    cycle(&env, &b, &config).await;
    assert!(read(b.config.join("forge/mcp.toml"))
        .unwrap()
        .contains("docs"));
    assert!(read(b.config.join("forge/marketplaces.toml"))
        .expect("marketplaces.toml is missing")
        .contains("acme"));

    write(a.config.join("forge/mcp.toml"), good);
    write(
        a.config.join("forge/marketplaces.toml"),
        "[marketplaces.acme]\nurl = \"https://example.com/m\"\n",
    );
    cycle(&env, &a, &config).await;
    assert!(clocks(&a).iter().all(|(_, c)| *c == 1), "{:?}", clocks(&a));
}

#[test]
fn a_failed_skill_write_leaves_the_previous_copy_whole() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("skills/foo");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join("SKILL.md"), "old").unwrap();
    std::fs::write(target.join("extra.md"), "old extra").unwrap();
    // "a" is written as a file and then "a/b" needs it to be a directory, so the write fails halfway.
    let value = serde_json::json!({"files":[
        {"path":"SKILL.md","b64":"bmV3"},
        {"path":"a","b64":"eA=="},
        {"path":"a/b","b64":"eQ=="},
    ]});
    assert!(super::files::write_bundle(&target, &value, false).is_err());
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "old"
    );
    assert_eq!(
        std::fs::read_to_string(target.join("extra.md")).unwrap(),
        "old extra"
    );
    let leftovers: Vec<_> = std::fs::read_dir(target.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        leftovers.len(),
        1,
        "no staging directory may remain: {leftovers:?}"
    );

    let ok = serde_json::json!({"files":[{"path":"SKILL.md","b64":"bmV3"}]});
    super::files::write_bundle(&target, &ok, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(target.join("SKILL.md")).unwrap(),
        "new"
    );
    assert!(!target.join("extra.md").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writes_to_one_id_converge_and_keep_the_loser_as_a_conflict_copy() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let config = forge_config::AccountSyncConfig {
        instructions: true,
        ..Default::default()
    };
    let a = machine(0xA1);
    let b = machine(0xB2);
    write(a.config.join("forge/AGENTS.md"), "from a");
    write(b.config.join("forge/AGENTS.md"), "from b");
    cycle(&env, &a, &config).await;
    cycle(&env, &b, &config).await;
    settle(&env, &mut Feed::default(), &[&a, &b], &config).await;

    let winner = read(a.config.join("forge/AGENTS.md"));
    assert_eq!(winner, read(b.config.join("forge/AGENTS.md")));
    let loser_copy = ["a", "b"]
        .iter()
        .zip([&a, &b])
        .filter_map(|(_, m)| read(m.config.join("forge/AGENTS.md.conflict")))
        .collect::<Vec<_>>();
    let other = if winner.as_deref() == Some("from a") {
        "from b"
    } else {
        "from a"
    };
    assert!(
        loser_copy.iter().any(|c| c == other),
        "the losing edit must survive as a conflict copy: {loser_copy:?}"
    );
}

#[test]
fn a_rejected_revision_retires_the_row_and_resubmits_the_winning_record() {
    let store = forge_store::Store::open_in_memory().unwrap();
    store.set_sync_journal_enabled(true).unwrap();
    let me = [0xB2; 16];
    store
        .write_portable_sync_record(me, "skill", "id1", Some(b"{\"x\":1}"))
        .unwrap();
    let entry = store.pending_sync_journal(10).unwrap().remove(0);
    assert_eq!(entry.revision, 1);
    assert!(sync_conflict::supersede(&store, me, &entry, 5).unwrap());
    let pending = store.pending_sync_journal(10).unwrap();
    assert_eq!(pending.len(), 1, "the drain must move on to a fresh row");
    assert_eq!(pending[0].revision, 2);

    store
        .write_portable_sync_record([0xA1; 16], "skill", "id2", Some(b"{\"x\":2}"))
        .unwrap();
    let entry = store
        .pending_sync_journal(10)
        .unwrap()
        .into_iter()
        .find(|e| e.stable_id == "id2")
        .unwrap();
    assert!(sync_conflict::supersede(&store, me, &entry, 5).unwrap());
    assert!(
        store
            .pending_sync_journal(10)
            .unwrap()
            .iter()
            .all(|e| e.stable_id != "id2"),
        "a record this device no longer owns is simply retired"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deferred_mcp_apply_does_not_make_the_local_copy_look_edited() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let config = forge_config::AccountSyncConfig {
        mcp: true,
        ..Default::default()
    };
    let server = |args: &str, extra: &str| {
        format!(
            "[[servers]]\nname = \"gh\"\n[servers.transport]\ntype = \"stdio\"\ncommand = \"gh-mcp-not-installed\"\nargs = [\"{args}\"]\n{extra}"
        )
    };
    let a = machine(0xA1);
    let b = machine(0xB2);
    write(a.config.join("forge/mcp.toml"), &server("one", ""));
    write(b.config.join("forge/mcp.toml"), &server("one", ""));
    cycle(&env, &a, &config).await;
    transfer(&a, &b);
    cycle(&env, &b, &config).await;
    let before = b.store.account_sync_items(Some("mcp")).unwrap();
    assert!(before.iter().any(|r| !r.local_hash.is_empty()));

    std::thread::sleep(std::time::Duration::from_millis(5));
    write(a.config.join("forge/mcp.toml"), &server("two", ""));
    cycle(&env, &a, &config).await;
    transfer(&a, &b);
    std::thread::sleep(std::time::Duration::from_millis(5));
    write(
        b.config.join("forge/mcp.toml"),
        &format!(
            "{}\n[[servers]]\nname = \"other\"\n[servers.transport]\ntype = \"http\"\nurl = \"https://example.com/o\"\n",
            server("one", "")
        ),
    );
    cycle(&env, &b, &config).await;
    cycle(&env, &b, &config).await;

    let row = b
        .store
        .account_sync_items(Some("mcp"))
        .unwrap()
        .into_iter()
        .find(|r| r.item == "server/gh")
        .unwrap();
    assert_eq!(row.status, "skipped");
    assert!(
        !row.local_hash.is_empty(),
        "the synced hash must survive a deferral"
    );
    let record = b
        .store
        .portable_sync_records("user_setting")
        .unwrap()
        .into_iter()
        .filter_map(|r| {
            let p: super::Payload = serde_json::from_slice(&r.payload).ok()?;
            (p.item == "server/gh").then_some(r.sender_device_id)
        })
        .next()
        .unwrap();
    assert_eq!(
        record, [0xA1; 16],
        "B must not upload its stale copy over A's edit"
    );
}

struct Rng(u64);

impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn random_interleavings_converge_without_losing_an_edit() {
    let _lock = crate::serve::tests::FORGE_DB_LOCK.lock().await;
    let env = Env::save();
    let config = forge_config::AccountSyncConfig {
        skills: true,
        instructions: true,
        ..Default::default()
    };
    let devices = [
        machine_with_epochs(0xA1, &[(1, 0x44), (2, 0x55)]),
        machine_with_epochs(0xB2, &[(2, 0x55)]),
        machine_with_epochs(0xC3, &[(1, 0x44), (2, 0x55)]),
    ];
    let refs: Vec<&Machine> = devices.iter().collect();
    let files = [
        "forge/AGENTS.md",
        "forge/skills/foo/SKILL.md",
        "forge/skills/bar/SKILL.md",
        "forge/commands/ship.md",
    ];
    let mut feed = Feed::default();
    let mut rng = Rng(0x5eed_f00d);
    let mut written: Vec<Vec<String>> = vec![Vec::new(); files.len()];
    let mut deleted = vec![false; files.len()];
    for step in 0..120 {
        let d = rng.next(devices.len());
        match rng.next(5) {
            0 | 1 => {
                let f = rng.next(files.len());
                let value = format!("d{d}-s{step}");
                write(devices[d].config.join(files[f]), &value);
                written[f].push(value);
            }
            2 if step > 20 => {
                let f = rng.next(files.len());
                let path = devices[d].config.join(files[f]);
                if path.exists() {
                    std::fs::remove_file(&path).unwrap();
                    if let Some(dir) = path
                        .parent()
                        .filter(|p| p.ends_with("foo") || p.ends_with("bar"))
                    {
                        std::fs::remove_dir_all(dir).unwrap();
                    }
                    deleted[f] = true;
                }
            }
            3 => {
                feed.upload(&devices[d]);
                feed.download(&devices[d]);
                cycle(&env, &devices[d], &config).await;
            }
            _ => cycle(&env, &devices[d], &config).await,
        }
    }
    settle(&env, &mut feed, &refs, &config).await;

    let snapshot = |m: &Machine| -> Vec<Option<String>> {
        files.iter().map(|f| read(m.config.join(f))).collect()
    };
    let first = snapshot(&devices[0]);
    for m in &devices[1..] {
        assert_eq!(snapshot(m), first, "devices must converge");
    }
    let before: Vec<_> = devices.iter().map(clocks).collect();
    round(&env, &mut feed, &refs, &config).await;
    let after: Vec<_> = devices.iter().map(clocks).collect();
    assert_eq!(before, after, "a converged fleet must be quiet");
    for (i, value) in first.iter().enumerate() {
        match value {
            Some(v) => assert!(
                written[i].contains(v),
                "{}: {v:?} was never written by anyone",
                files[i]
            ),
            None => assert!(
                deleted[i] || written[i].is_empty(),
                "{} vanished without a delete",
                files[i]
            ),
        }
    }
}
