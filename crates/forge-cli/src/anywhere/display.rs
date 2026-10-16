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
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| raw.to_string())
}

/// `[anywhere] host_name` when set, else the system hostname.
pub(super) fn configured_host_name() -> String {
    forge_config::load()
        .ok()
        .and_then(|config| config.anywhere.host_name)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(super::default_host_name)
}
