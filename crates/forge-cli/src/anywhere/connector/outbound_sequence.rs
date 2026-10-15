//! Block-based outbound sequence reservation for the connector's hot path.
//!
//! Reserving per envelope took an exclusive file lock plus a full state rewrite with two fsyncs.
//! Instead the persisted `next_sequence` is advanced by a whole block under the lock and the
//! block is handed out from memory. A crash only skips the unused tail, never reuses a sequence,
//! and other processes reserving from the file start after the block.

use super::{identity_from_state, Identity, StateStore};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

pub(super) const OUTBOUND_BLOCK: usize = 256;
// The identity is cached with its block; bound how long a rotated key or epoch can go unnoticed.
const MAX_BLOCK_AGE: Duration = Duration::from_secs(30);

struct CachedBlock {
    identity: Arc<Identity>,
    next: u64,
    limit: u64,
    loaded_at: Instant,
}

static BLOCKS: LazyLock<Mutex<HashMap<PathBuf, CachedBlock>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn reserve_outbound(store: &StateStore) -> Result<(Arc<Identity>, u64)> {
    reserve_outbound_with(store, OUTBOUND_BLOCK, MAX_BLOCK_AGE)
}

fn reserve_outbound_with(
    store: &StateStore,
    block: usize,
    max_age: Duration,
) -> Result<(Arc<Identity>, u64)> {
    let mut blocks = BLOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(cached) = blocks.get_mut(&store.path) {
        if cached.next < cached.limit && cached.loaded_at.elapsed() < max_age {
            let sequence = cached.next;
            cached.next += 1;
            return Ok((Arc::clone(&cached.identity), sequence));
        }
    }
    let (state, first) = super::super::state::blocking_fs(|| store.reserve_sequences(block))?;
    let identity = Arc::new(identity_from_state(&state)?);
    blocks.insert(
        store.path.clone(),
        CachedBlock {
            identity: Arc::clone(&identity),
            next: first + 1,
            limit: first + block as u64,
            loaded_at: Instant::now(),
        },
    );
    Ok((identity, first))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anywhere::LocalState;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    fn store(dir: &std::path::Path) -> StateStore {
        let store = StateStore {
            path: dir.join("anywhere/state.json"),
        };
        store
            .save(&LocalState {
                version: crate::anywhere::STATE_VERSION,
                account_id: Some(hex::encode([0x11; 16])),
                device_id: Some(hex::encode([0x22; 16])),
                signing_private_key: Some(URL_SAFE_NO_PAD.encode([0x44; 32])),
                account_data_key: Some(URL_SAFE_NO_PAD.encode([0x55; 32])),
                key_epoch: Some(3),
                host_id: Some(hex::encode([0x33; 16])),
                ..LocalState::default()
            })
            .expect("save state");
        store
    }

    #[test]
    fn persists_once_per_block_and_never_reuses_across_exhaustion() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = store(temp.path());
        let mut seen = Vec::new();
        for _ in 0..7 {
            seen.push(
                reserve_outbound_with(&store, 3, Duration::from_secs(60))
                    .unwrap()
                    .1,
            );
        }
        assert_eq!(seen, vec![0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(store.load().unwrap().next_sequence, 9);
    }

    #[test]
    fn another_process_reserving_between_blocks_cannot_collide() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = store(temp.path());
        let first = reserve_outbound_with(&store, 4, Duration::from_secs(60))
            .unwrap()
            .1;
        let (_, other) = store.reserve_sequences(1).unwrap();
        assert_eq!(first, 0);
        assert_eq!(other, 4);
        let mut later = Vec::new();
        for _ in 0..6 {
            later.push(
                reserve_outbound_with(&store, 4, Duration::from_secs(60))
                    .unwrap()
                    .1,
            );
        }
        assert_eq!(later, vec![1, 2, 3, 5, 6, 7]);
    }

    #[test]
    fn expired_block_is_reloaded_without_reusing_its_tail() {
        let temp = tempfile::tempdir().expect("temp dir");
        let store = store(temp.path());
        assert_eq!(
            reserve_outbound_with(&store, 8, Duration::ZERO).unwrap().1,
            0
        );
        assert_eq!(
            reserve_outbound_with(&store, 8, Duration::ZERO).unwrap().1,
            8
        );
    }
}
