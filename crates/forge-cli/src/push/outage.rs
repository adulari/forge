/// An outage shorter than this is a blip that clears itself; only a longer one is worth a push.
pub(crate) const OUTAGE_PUSH_AFTER_SECS: u64 = 120;

/// Test-only override of [`OUTAGE_PUSH_AFTER_SECS`], so a layer-2 test need not sit out two minutes.
#[cfg(test)]
pub(crate) static OUTAGE_PUSH_AFTER_TEST: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(u64::MAX);

fn outage_push_after_secs() -> u64 {
    #[cfg(test)]
    {
        let over = OUTAGE_PUSH_AFTER_TEST.load(std::sync::atomic::Ordering::SeqCst);
        if over != u64::MAX {
            return over;
        }
    }
    OUTAGE_PUSH_AFTER_SECS
}

/// One outage push and one recovery push per turn, whatever number of probes the wait makes.
#[derive(Debug, Default)]
pub(crate) struct OutagePusher {
    announced: bool,
    outstanding: bool,
}

impl OutagePusher {
    /// A new turn begins: its first long outage may push again.
    pub(crate) fn new_turn(&mut self) {
        *self = Self::default();
    }

    /// A probe of the wait at `now_unix`, for an outage that began at `since_unix`. True exactly
    /// once per turn: the first probe after the outage has lasted [`OUTAGE_PUSH_AFTER_SECS`].
    pub(crate) fn outage_probe(&mut self, now_unix: u64, since_unix: u64) -> bool {
        self.outage_probe_after(now_unix, since_unix, outage_push_after_secs())
    }

    pub(super) fn outage_probe_after(
        &mut self,
        now_unix: u64,
        since_unix: u64,
        after_secs: u64,
    ) -> bool {
        if self.announced || now_unix.saturating_sub(since_unix) < after_secs {
            return false;
        }
        self.announced = true;
        self.outstanding = true;
        true
    }

    /// The provider is back. True only when the outage was announced, so a recovery nobody was
    /// told about the start of is not pushed either.
    pub(crate) fn recovered(&mut self) -> bool {
        std::mem::take(&mut self.outstanding)
    }
}
