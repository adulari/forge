//! Drive an Android phone or emulator over adb.
//!
//! The mobile counterpart to `forge-browser`: where that crate drives a real Chrome through the
//! DevTools Protocol, this drives a real device through adb. The shape is deliberately the same —
//! a control surface ([`Device`]) and an inspection surface ([`logcat`]) — because the jobs are
//! the same: act on the thing, then read what it did.
//!
//! Pair it with `forge-proxy` for app testing: [`net::set_proxy`] and [`net::install_system_ca`]
//! point the device's traffic at Forge's proxy and make it trust the interception, so a request an
//! app makes is both observable and rewritable while the app is being driven.
//!
//! Nothing here holds a persistent connection: adb is a request/response CLI and every call
//! re-invokes it, so a device that disappears mid-session produces a clean error rather than a
//! stale handle.

pub mod adb;
pub mod emulator;
pub mod logcat;
pub mod net;
pub mod session;
pub mod ui;

pub use adb::{Adb, DeviceInfo};
pub use emulator::BootOptions;
pub use logcat::{Filter, Level, LogLine};
pub use session::{Device, DeviceProfile};
pub use ui::{Bounds, Hierarchy, Selector, UiNode};

/// Locate adb and bind to a device, resolving the serial the way every entry point should:
/// an explicit serial if given, the only connected device otherwise.
pub async fn connect(serial: Option<&str>) -> anyhow::Result<Device> {
    let adb = Adb::locate()?;
    Ok(Device::new(adb.resolve(serial).await?))
}

/// Truncate long output for a transcript, keeping the head and saying what was cut.
pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let kept: String = text.chars().take(max_chars).collect();
    let dropped = text.chars().count() - max_chars;
    format!("{kept}\n… truncated, {dropped} more characters")
}

#[cfg(test)]
mod tests {
    #[test]
    fn truncate_keeps_short_text_intact() {
        assert_eq!(super::truncate("short", 100), "short");
    }

    #[test]
    fn truncate_reports_what_it_dropped() {
        let out = super::truncate("abcdef", 3);
        assert!(out.starts_with("abc"));
        assert!(out.contains("3 more characters"));
    }
}
