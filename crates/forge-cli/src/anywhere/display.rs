/// Render a service timestamp: epoch seconds or milliseconds become UTC `YYYY-MM-DD HH:MM`;
/// anything else (already formatted) passes through.
pub(super) fn display_timestamp(raw: &str) -> String {
    let Ok(value) = raw.trim().parse::<i64>() else {
        return raw.to_string();
    };
    let seconds = if value > 100_000_000_000 {
        value / 1000
    } else {
        value
    };
    crate::timefmt::utc(seconds, "%Y-%m-%d %H:%M UTC").unwrap_or_else(|| raw.to_string())
}

/// `[anywhere] host_name` when set, else the system hostname.
pub(super) fn configured_host_name() -> String {
    forge_config::load()
        .ok()
        .and_then(|config| config.anywhere.host_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(super::default_host_name)
}

pub(super) fn human_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

#[cfg(test)]
mod time_pins {
    use super::*;

    #[test]
    fn service_timestamps_render_as_utc_minutes_and_text_passes_through() {
        assert_eq!(display_timestamp("1700000000"), "2023-11-14 22:13 UTC");
        assert_eq!(display_timestamp("1700000000999"), "2023-11-14 22:13 UTC");
        assert_eq!(display_timestamp(" 0 "), "1970-01-01 00:00 UTC");
        assert_eq!(display_timestamp("yesterday"), "yesterday");
        assert_eq!(
            display_timestamp("2026-01-01T00:00:00Z"),
            "2026-01-01T00:00:00Z"
        );
    }
}
