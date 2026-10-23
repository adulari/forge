//! The current wall-clock time, as the model sees it.
//!
//! Forge told the model WHERE it was — `working_directory`, `platform`, `git_branch` in the
//! `<env>` block — but never WHEN. Nothing in [`crate::FORGE_SYSTEM`], the system preamble, or any
//! registered tool carried a date, and no provider injected one. With no signal at all a model
//! answers "today" from its training cutoff, so it mis-dates changelog and release entries, reasons
//! wrongly about how old a commit or a dependency version is, and reads "recent" / "latest" as
//! whatever was recent when it was trained.
//!
//! Injected per TURN rather than into the system preamble, deliberately. The preamble is the
//! provider prompt-cache anchor (see [`crate::Session::system_preamble`]) — "placed first so the
//! provider's cache breakpoint anchors on this stable prefix". A clock that changes every second
//! sitting in that prefix would invalidate the cached prefix on EVERY request, which on a long
//! session is a large and permanent cost and latency regression. As turn context it costs a few
//! tokens once per turn, keeps the cache warm, and leaves a temporal trail in the transcript so the
//! model can also see how long ago an earlier turn happened.

/// The turn-start time line handed to the model.
pub(crate) fn now_line() -> String {
    line_for(chrono::Local::now())
}

/// The clock line for a session that has already been told the full instruction: just the time.
/// The ~280 chars of "treat this as the present moment" are the same every turn, and the first
/// copy is still in the transcript.
pub(crate) fn short_line() -> String {
    short_line_for(chrono::Local::now())
}

/// Whether `content` is the full clock instruction (as opposed to the short per-turn line).
pub(crate) fn is_full_line(content: &str) -> bool {
    content.starts_with(FULL_LINE_PREFIX)
}

const FULL_LINE_PREFIX: &str = "[time] Current date and time: ";
/// To the minute. On a CLI bridge the line sits in the first user message, ahead of the prompt;
/// with seconds in it no two first requests were ever byte-identical, so the provider wrote that
/// whole message (2.1k tokens on a trivial `claude-cli::haiku` turn) to its cache every time
/// instead of reading it back as Claude Code's own date-only prompt is.
const TIMESTAMP_FORMAT: &str = "%A %Y-%m-%d %H:%M %:z";

fn short_line_for(now: chrono::DateTime<chrono::Local>) -> String {
    format!("[time] {}", now.format(TIMESTAMP_FORMAT))
}

/// Pure formatter, so the wording and shape are testable without depending on the wall clock.
fn line_for(now: chrono::DateTime<chrono::Local>) -> String {
    format!(
        "{FULL_LINE_PREFIX}{}. Treat this as the present moment — your training \
         cutoff is NOT today. Use it for anything dated: changelog and release entries, how old a \
         commit, file or dependency version is, and what \"today\", \"now\", \"recent\" or \
         \"latest\" mean. Do not guess the date, and do not assume a year from your training data.",
        now.format(TIMESTAMP_FORMAT)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_line_carries_a_full_local_timestamp() {
        let at = chrono::Local
            .with_ymd_and_hms(2026, 9, 14, 1, 20, 33)
            .single()
            .expect("unambiguous local time");
        let line = line_for(at);
        assert!(line.contains("2026-09-14"), "{line}");
        assert!(line.contains("01:20 "), "{line}");
        assert!(
            !line.contains("01:20:33"),
            "seconds would break prompt caching: {line}"
        );
        // The weekday matters: "is the 14th a Monday" is exactly the kind of thing a model
        // otherwise derives from a wrong year.
        assert!(line.contains("Monday"), "{line}");
    }

    #[test]
    fn the_line_tells_the_model_to_prefer_it_over_its_training_cutoff() {
        let at = chrono::Local
            .with_ymd_and_hms(2026, 9, 14, 1, 20, 33)
            .single()
            .expect("unambiguous local time");
        let line = line_for(at);
        assert!(line.contains("present moment"), "{line}");
        assert!(line.contains("training"), "{line}");
    }

    #[test]
    fn the_short_line_carries_the_same_timestamp_without_the_instruction() {
        let at = chrono::Local
            .with_ymd_and_hms(2026, 9, 14, 1, 20, 33)
            .single()
            .expect("unambiguous local time");
        let short = short_line_for(at);
        assert!(
            short.starts_with("[time] Monday 2026-09-14 01:20 "),
            "{short}"
        );
        assert!(!is_full_line(&short));
        assert!(is_full_line(&line_for(at)));
        assert!(short.len() * 3 < line_for(at).len());
    }

    #[test]
    fn the_live_clock_produces_a_usable_line() {
        let line = now_line();
        assert!(line.starts_with("[time] Current date and time: "), "{line}");
        // A timezone offset is present, so relative reasoning ("in 3 hours") is well-defined.
        assert!(
            line.contains('+') || line.contains('-'),
            "expected a UTC offset: {line}"
        );
    }
}
