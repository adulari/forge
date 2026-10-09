//! The bridge turn file: a long-lived bridge child takes its per-turn seq from the file the
//! parent rewrites each user turn, falling back to `FORGE_CHECKPOINT_SEQ`. Own test binary
//! because it mutates process-global env.

use forge_core::snapshot::{
    current_seq, read_bridge_turn, record_from_env_after_write, restore_turn,
    snapshot_from_env_before_write, ENV_ROOT, ENV_SEQ, ENV_SESSION, ENV_TURN_FILE,
};

#[test]
fn snapshot_seq_follows_the_turn_file() {
    let root = std::env::temp_dir().join(format!("forge-turnfile-{}", forge_types::new_id()));
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let turn_file = root.join("turn.json");

    std::env::set_var(ENV_SESSION, "s");
    std::env::set_var(ENV_SEQ, "1");
    std::env::set_var(ENV_ROOT, root.to_str().unwrap());

    // No turn-file var: env seq wins.
    assert_eq!(current_seq(), Some(1));

    std::env::set_var(ENV_TURN_FILE, &turn_file);
    // Unreadable file: fall back to env.
    assert_eq!(read_bridge_turn(), None);
    assert_eq!(current_seq(), Some(1));

    std::fs::write(&turn_file, r#"{"seq": 5, "mode": "bypass"}"#).unwrap();
    assert_eq!(current_seq(), Some(5));
    assert_eq!(read_bridge_turn().unwrap().mode.as_deref(), Some("bypass"));

    let file = work.join("a.rs");
    std::fs::write(&file, "PRE").unwrap();
    snapshot_from_env_before_write(&file).unwrap();
    std::fs::write(&file, "EDIT").unwrap();
    record_from_env_after_write(&file).unwrap();
    assert!(!restore_turn(&root, "s", 5).unwrap().restored.is_empty());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "PRE");

    std::fs::write(&turn_file, "{garbage").unwrap();
    assert_eq!(current_seq(), Some(1), "unparsable file falls back to env");

    std::fs::remove_dir_all(&root).ok();
}
