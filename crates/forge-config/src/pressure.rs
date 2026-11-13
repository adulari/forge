//! `[pressure]`: when a `forge serve` daemon tells its owner the host is running out of disk or
//! memory, and when it stops starting new worktree sessions. A dev box that silently fills up
//! shows itself as a confusing build failure hours later; the daemon samples its own host and
//! says so first.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PressureConfig {
    /// Sample the host and report pressure. `false` silences the warning, the push and the
    /// refusal together.
    pub enabled: bool,
    /// Seconds between samples.
    pub sample_secs: u64,
    /// Warn when a filesystem the daemon uses has less than this many GB free...
    pub disk_warn_free_gb: u64,
    /// ...or less than this percent of its capacity free.
    pub disk_warn_free_percent: u8,
    /// Refuse new worktree sessions below this many GB free...
    pub disk_floor_free_gb: u64,
    /// ...or below this percent of capacity free.
    pub disk_floor_free_percent: u8,
    /// Warn when available memory drops below this many MB...
    pub memory_low_mb: u64,
    /// ...or below this percent of total memory.
    pub memory_low_percent: u8,
}

impl Default for PressureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            sample_secs: 60,
            disk_warn_free_gb: 20,
            disk_warn_free_percent: 10,
            disk_floor_free_gb: 3,
            disk_floor_free_percent: 2,
            memory_low_mb: 1024,
            memory_low_percent: 5,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_on_and_a_partial_table_keeps_the_rest() {
        let parsed: PressureConfig = toml::from_str("").unwrap();
        assert_eq!(parsed, PressureConfig::default());
        let set: PressureConfig = toml::from_str("disk_warn_free_gb = 50").unwrap();
        assert_eq!(set.disk_warn_free_gb, 50);
        assert_eq!(set.disk_floor_free_gb, 3);
        assert!(set.enabled);
    }
}
