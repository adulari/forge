//! The panic hook: capture, durable local write, then a bounded attempt to deliver before the
//! process goes away.

use std::cell::Cell;
use std::panic::PanicHookInfo;
use std::sync::Once;
use std::time::Duration;

use crate::report::ErrorKind;
use crate::{frames, scrub, Incident, ReportingMode};

thread_local! {
    static IN_HOOK: Cell<bool> = const { Cell::new(false) };
}

fn payload_text(info: &PanicHookInfo<'_>) -> String {
    info.payload()
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| info.payload().downcast_ref::<&str>().copied())
        .unwrap_or("panic with a non-string payload")
        .to_string()
}

/// Build the incident for a panic. Separated from the hook so it is testable.
pub(crate) fn incident_for(
    message: String,
    location: Option<String>,
    backtrace: &str,
    thread_name: Option<&str>,
) -> Incident {
    let parsed = frames::parse_backtrace(backtrace);
    let normalized = location.as_deref().map(scrub::normalize_location);
    let file_only = normalized
        .as_deref()
        .map(|l| l.rsplit_once(':').map_or(l, |(file, _)| file).to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let render = file_only.starts_with("forge-tui/src/render")
        || parsed
            .frames
            .iter()
            .take(6)
            .any(|f| f.contains("::render") || f.contains("::draw"));
    let kind = if render {
        ErrorKind::RenderPanic
    } else if thread_name.is_some_and(|n| n.starts_with("tokio-runtime-worker")) {
        ErrorKind::TaskDeath
    } else {
        ErrorKind::Panic
    };
    Incident {
        kind,
        site: file_only,
        message,
        frames: parsed.frames,
        location: normalized.or(parsed.location),
        check_text: true,
    }
}

fn capture(info: &PanicHookInfo<'_>) {
    if IN_HOOK.with(|flag| flag.replace(true)) {
        return;
    }
    let message = payload_text(info);
    let location = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()));
    let backtrace = std::backtrace::Backtrace::force_capture().to_string();
    let thread = std::thread::current();
    let incident = incident_for(message, location, &backtrace, thread.name());
    if let crate::Disposition::Queued = crate::report(incident) {
        if crate::mode_cached() == ReportingMode::Auto {
            let _ = crate::flush_blocking(Duration::from_secs(2));
        }
    }
    IN_HOOK.with(|flag| flag.set(false));
}

/// Install the reporting panic hook, chained after whatever hook is already installed (so the
/// terminal is restored and the message printed exactly as before). Idempotent.
pub fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture(info)));
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_panics_and_task_deaths_get_their_own_kinds() {
        let render = incident_for(
            "index out of bounds".into(),
            Some("crates/forge-tui/src/render/transcript.rs:88".into()),
            "",
            Some("main"),
        );
        assert_eq!(render.kind, ErrorKind::RenderPanic);
        let task = incident_for(
            "boom".into(),
            Some("crates/forge-cli/src/serve.rs:10".into()),
            "",
            Some("tokio-runtime-worker"),
        );
        assert_eq!(task.kind, ErrorKind::TaskDeath);
        let plain = incident_for(
            "boom".into(),
            Some("crates/forge-core/src/lib.rs:10".into()),
            "",
            Some("main"),
        );
        assert_eq!(plain.kind, ErrorKind::Panic);
        assert_eq!(plain.site, "forge-core/src/lib.rs");
        assert_eq!(plain.location.as_deref(), Some("forge-core/src/lib.rs:10"));
    }
}
