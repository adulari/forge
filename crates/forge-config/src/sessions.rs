//! `[sessions]`: how long the daemon keeps idle sessions loaded and when it archives stale ones.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionsConfig {
    /// Minutes a hosted session may sit idle before the daemon drops it from memory. Its history
    /// stays in the store and resuming re-hydrates it. `0` disables unloading.
    pub idle_unload_minutes: u32,
    /// Days of inactivity before a session that is not starred is archived. `0` disables it.
    pub auto_archive_days: u32,
}

impl Default for SessionsConfig {
    fn default() -> Self {
        Self {
            idle_unload_minutes: 60,
            auto_archive_days: 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_unload_after_an_hour_and_archive_after_three_days() {
        let parsed: SessionsConfig = toml::from_str("").unwrap();
        assert_eq!(parsed.idle_unload_minutes, 60);
        assert_eq!(parsed.auto_archive_days, 3);
        let off: SessionsConfig = toml::from_str("auto_archive_days = 0").unwrap();
        assert_eq!(off.auto_archive_days, 0);
        assert_eq!(off.idle_unload_minutes, 60);
    }
}
