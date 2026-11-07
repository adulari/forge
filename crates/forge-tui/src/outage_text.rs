//! One-line renderings of a provider outage for the surfaces that print lines (headless, print
//! mode) and the status bar. Times arrive as unix seconds and are shown in local time.

pub(crate) fn clock(secs: u64) -> String {
    chrono::DateTime::<chrono::Local>::from(
        std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs),
    )
    .format("%H:%M")
    .to_string()
}

pub(crate) fn outage_line(
    model: &str,
    since: u64,
    next_try: u64,
    until: u64,
    reason: &str,
) -> String {
    format!(
        "  · {} down since {} ({reason}) · next try {} · until {}",
        forge_config::provider_of(model),
        clock(since),
        clock(next_try),
        clock(until),
    )
}

pub(crate) fn recovered_line(model: &str, down_secs: u64) -> String {
    let span = match (down_secs / 3600, down_secs / 60 % 60) {
        (0, 0) => format!("{down_secs}s"),
        (0, m) => format!("{m}m"),
        (h, m) => format!("{h}h{m:02}m"),
    };
    format!("  · {model} back after {span}, turn resumed")
}

pub(crate) fn status_line(model: &str, since: u64, next_try: u64, until: u64) -> String {
    format!(
        "{} down since {} · next try {} · until {} · Esc stops",
        forge_config::provider_of(model),
        clock(since),
        clock(next_try),
        clock(until),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_recovery_line_reads_like_the_stored_note() {
        assert_eq!(
            recovered_line("inferhub::flash", 4 * 3600 + 9 * 60 + 5),
            "  · inferhub::flash back after 4h09m, turn resumed"
        );
        assert_eq!(
            recovered_line("inferhub::flash", 253),
            "  · inferhub::flash back after 4m, turn resumed"
        );
        assert_eq!(
            recovered_line("inferhub::flash", 40),
            "  · inferhub::flash back after 40s, turn resumed"
        );
    }
}
