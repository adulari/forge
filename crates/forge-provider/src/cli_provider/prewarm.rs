//! Start a claude bridge process before the first prompt, so that prompt does not pay for it.
//!
//! A first turn on the persistent transport spawns `claude`, which starts `forge mcp-serve`, and
//! waits for the tool server to report connected before the prompt can be written. That is the
//! better part of a second the person watches as "nothing happens". Doing the same work while they
//! type moves it off the first token. The warmed process is parked in the conversation's slot
//! marked [`LiveSession::unused`]; a turn that matches it takes it over exactly like a fresh spawn.

use super::*;

/// How long a turn waits for its conversation's bridge process to finish warming: the warm-up's own
/// handshake bound plus its spawn.
pub(super) const PREWARM_WAIT: Duration = Duration::from_secs(40);

/// Ends the owner's warm-up mark however `prewarm_live` returns.
struct Warming<'a> {
    live: &'a std::sync::Mutex<live::LiveMap>,
    owner: &'a str,
}

impl Drop for Warming<'_> {
    fn drop(&mut self) {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .end_warming(self.owner);
    }
}

impl CliProvider {
    pub(super) fn is_warming(&self, owner: Option<&str>) -> bool {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_warming(owner)
    }

    /// Warm a live claude process for the conversation `checkpoint` names. A hint: it does nothing
    /// when the transport is not the persistent claude harness, when a process already lives in the
    /// slot, or when anything fails (the first turn then spawns as it always did). It holds the slot
    /// for the whole warm-up and marks the owner as warming, so a prompt that arrives meanwhile
    /// waits for this process rather than spawning a second one.
    pub(super) async fn prewarm_live(
        &self,
        model: &str,
        checkpoint: &CheckpointContext,
        effort: Option<EffortLevel>,
    ) {
        if !(self.persistent && self.kind == CliKind::ClaudeCode && self.harness) {
            return;
        }
        let owner = checkpoint.session.as_str();
        let slot = {
            let mut live = self
                .live
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !live.begin_warming(owner) {
                return;
            }
            live.slot_for(Some(owner))
        };
        let _warming = Warming {
            live: &self.live,
            owner,
        };
        let Ok(mut guard) = slot.try_lock() else {
            return;
        };
        if guard.session.is_some() {
            return;
        }
        let mut session = match self
            .spawn_live(bare_model(model), Some(checkpoint), effort, None)
            .await
        {
            Ok(session) => session,
            Err(error) => {
                tracing::debug!(target: "forge::turn", "bridge prewarm spawn failed: {error}");
                return;
            }
        };
        let startup_timeout = self.timeout.min(Duration::from_secs(30));
        if let Err(error) = session.initialize_forge_mcp(startup_timeout).await {
            tracing::debug!(target: "forge::turn", "bridge prewarm handshake failed: {error}");
            session.teardown().await;
            return;
        }
        session.needs_init = false;
        session.unused = true;
        session.parked_at = std::time::Instant::now();
        let pid = session.child.id();
        guard.session = Some(session);
        drop(guard);
        tracing::debug!(target: "forge::turn", "bridge prewarmed");
        arm_live_idle_reaper(slot, pid, self.live_idle_ttl);
    }
}
