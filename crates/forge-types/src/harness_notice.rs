//! Recognises every user-role message the harness injects that the person did not type, so each
//! surface can show it as a harness notice instead of a "you" row. The model-facing text is
//! unchanged; this only reads it back.
//!
//! The generators (`forge_core::wakeup`, `forge_core::heartbeat`, `forge_core::fleet`,
//! `forge_core::job_wake`) build their text with the format functions and constants below, so a
//! generator and this parser cannot drift apart. Matching is strict on those exact shapes. The one
//! unavoidable false positive: a person who literally types text in one of these shapes (for
//! example `[heartbeat] x`) sees it rendered as a notice; the model transcript is unaffected.
//!
//! Not covered because they are not user-role rows the person could mistake for their own: nudges
//! (stored with the `nudge` flag and already tagged `kind: "nudge"`) and detached-subagent results
//! (stored `system`).

use crate::job_notice::{parse_job_notices, JobNotice};

/// Every delivered fleet message starts with this.
pub const PEER_MESSAGE_PREFIX: &str = "[message from ";
/// Separates a peer message's framing from its body.
pub const PEER_BODY_SEPARATOR: &str = "\n---\n";
pub const WAKEUP_PREFIX: &str = "[scheduled wakeup] ";
const WAKEUP_REASON_OPEN: &str = "\n\n(scheduled because: ";
const HEARTBEAT_PREFIX: &str = "[heartbeat";
const HEARTBEAT_LABEL_MAX: usize = 40;

/// What a wakeup's stored prompt looks like: the agent's prompt plus why it was scheduled.
pub fn format_wakeup_prompt(prompt: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        prompt.trim().to_string()
    } else {
        format!("{}{WAKEUP_REASON_OPEN}{reason})", prompt.trim())
    }
}

/// The turn text delivered when a wakeup fires (`stored_prompt` is [`format_wakeup_prompt`]).
pub fn format_wakeup_delivery(stored_prompt: &str) -> String {
    format!("{WAKEUP_PREFIX}{stored_prompt}")
}

/// The turn text delivered when a user `/heartbeat` fires.
pub fn format_heartbeat_delivery(label: Option<&str>, prompt: &str) -> String {
    match label {
        Some(label) => format!("{HEARTBEAT_PREFIX}: {label}] {prompt}"),
        None => format!("{HEARTBEAT_PREFIX}] {prompt}"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessNotice {
    Job(Vec<JobNotice>),
    /// An agent-scheduled wakeup firing.
    Wakeup {
        prompt: String,
        reason: Option<String>,
    },
    /// A user `/heartbeat` recurring prompt firing.
    Heartbeat {
        label: Option<String>,
        prompt: String,
    },
    /// A message another agent session sent.
    Peer {
        who: String,
        body: String,
    },
}

impl HarnessNotice {
    /// The wire subtype word: remote rows of `kind: "harness_notice"` carry it as `meta`.
    pub fn subtype(&self) -> &'static str {
        match self {
            Self::Job(_) => "job",
            Self::Wakeup { .. } => "wakeup",
            Self::Heartbeat { .. } => "heartbeat",
            Self::Peer { .. } => "peer",
        }
    }

    /// One dim line for non-job notices: `⏰ wakeup · <first line…>`. `None` for jobs, which have
    /// their own multi-line rendering.
    pub fn headline(&self, max_chars: usize) -> Option<String> {
        match self {
            Self::Job(_) => None,
            Self::Wakeup { prompt, .. } => {
                Some(format!("⏰ wakeup · {}", first_line(prompt, max_chars)))
            }
            Self::Heartbeat { label, prompt } => Some(format!(
                "♻ heartbeat{} · {}",
                label
                    .as_deref()
                    .map(|l| format!(" {l}"))
                    .unwrap_or_default(),
                first_line(prompt, max_chars)
            )),
            Self::Peer { who, .. } => Some(format!("✉ peer message from {who}")),
        }
    }
}

fn first_line(text: &str, max_chars: usize) -> String {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let line = line.trim();
    if line.chars().count() <= max_chars {
        line.to_string()
    } else {
        let cut: String = line.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{}…", cut.trim_end())
    }
}

pub fn parse_harness_notice(text: &str) -> Option<HarnessNotice> {
    if let Some(rest) = text.strip_prefix(WAKEUP_PREFIX) {
        return parse_wakeup(rest);
    }
    if let Some(rest) = text.strip_prefix(HEARTBEAT_PREFIX) {
        return parse_heartbeat(rest);
    }
    if let Some(rest) = text.strip_prefix(PEER_MESSAGE_PREFIX) {
        return parse_peer(rest);
    }
    parse_job_notices(text).map(HarnessNotice::Job)
}

fn parse_wakeup(rest: &str) -> Option<HarnessNotice> {
    if rest.trim().is_empty() {
        return None;
    }
    if let Some(body) = rest.strip_suffix(')') {
        if let Some((prompt, reason)) = body.rsplit_once(WAKEUP_REASON_OPEN) {
            if !prompt.trim().is_empty() && !reason.trim().is_empty() {
                return Some(HarnessNotice::Wakeup {
                    prompt: prompt.to_string(),
                    reason: Some(reason.to_string()),
                });
            }
        }
    }
    Some(HarnessNotice::Wakeup {
        prompt: rest.to_string(),
        reason: None,
    })
}

fn parse_heartbeat(rest: &str) -> Option<HarnessNotice> {
    if let Some(prompt) = rest.strip_prefix("] ") {
        return Some(HarnessNotice::Heartbeat {
            label: None,
            prompt: prompt.to_string(),
        });
    }
    let (label, prompt) = rest.strip_prefix(": ")?.split_once("] ")?;
    let valid = !label.is_empty()
        && label.len() <= HEARTBEAT_LABEL_MAX
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| HarnessNotice::Heartbeat {
        label: Some(label.to_string()),
        prompt: prompt.to_string(),
    })
}

fn parse_peer(rest: &str) -> Option<HarnessNotice> {
    let (who, tail) = rest.split_once("] ")?;
    if who.is_empty() || who.contains('\n') {
        return None;
    }
    let body = tail
        .split_once(PEER_BODY_SEPARATOR)
        .map_or(tail, |(_, body)| body);
    Some(HarnessNotice::Peer {
        who: who.to_string(),
        body: body.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakeup_with_and_without_reason_round_trips_the_generator_output() {
        let stored = format_wakeup_prompt("  Check the probe file  ", " waiting for Claude ");
        assert_eq!(
            parse_harness_notice(&format_wakeup_delivery(&stored)),
            Some(HarnessNotice::Wakeup {
                prompt: "Check the probe file".into(),
                reason: Some("waiting for Claude".into()),
            })
        );
        let bare = format_wakeup_prompt("ping", "");
        assert_eq!(
            parse_harness_notice(&format_wakeup_delivery(&bare)),
            Some(HarnessNotice::Wakeup {
                prompt: "ping".into(),
                reason: None
            })
        );
    }

    #[test]
    fn wakeup_prompt_may_be_multiline() {
        let stored = format_wakeup_prompt("line one\nline two", "r");
        let Some(HarnessNotice::Wakeup { prompt, reason }) =
            parse_harness_notice(&format_wakeup_delivery(&stored))
        else {
            panic!()
        };
        assert_eq!(prompt, "line one\nline two");
        assert_eq!(reason.as_deref(), Some("r"));
    }

    #[test]
    fn heartbeat_labelled_and_bare_round_trip() {
        assert_eq!(
            parse_harness_notice(&format_heartbeat_delivery(Some("ci-watch"), "check CI")),
            Some(HarnessNotice::Heartbeat {
                label: Some("ci-watch".into()),
                prompt: "check CI".into()
            })
        );
        assert_eq!(
            parse_harness_notice(&format_heartbeat_delivery(None, "check in")),
            Some(HarnessNotice::Heartbeat {
                label: None,
                prompt: "check in".into()
            })
        );
    }

    #[test]
    fn near_misses_stay_user_prompts() {
        for text in [
            "fix the bug",
            "",
            "[heartbeat: bad label] x",
            "[heartbeat: ] x",
            "[heartbeat",
            "[heartbeatx] y",
            "[scheduled wakeup] ",
            "please [scheduled wakeup] x",
            "[message from ] x",
        ] {
            assert_eq!(parse_harness_notice(text), None, "{text:?}");
        }
    }

    #[test]
    fn peer_messages_drop_the_framing() {
        let framed = "[message from session \"alpha\" (3fa91c2d)] This is a peer message.\n---\nplease check\nthanks";
        assert_eq!(
            parse_harness_notice(framed),
            Some(HarnessNotice::Peer {
                who: "session \"alpha\" (3fa91c2d)".into(),
                body: "please check\nthanks".into()
            })
        );
        assert_eq!(
            parse_harness_notice("[message from cli] hello"),
            Some(HarnessNotice::Peer {
                who: "cli".into(),
                body: "hello".into()
            })
        );
    }

    #[test]
    fn job_notices_are_still_recognised() {
        let n = parse_harness_notice("[background job 7 `make` exited 2]").unwrap();
        assert_eq!(n.subtype(), "job");
        assert!(n.headline(40).is_none());
    }

    #[test]
    fn headlines_are_dim_one_liners() {
        let w = HarnessNotice::Wakeup {
            prompt: "\nCheck the file now please".into(),
            reason: None,
        };
        assert_eq!(w.headline(11).unwrap(), "⏰ wakeup · Check the…");
        let h = HarnessNotice::Heartbeat {
            label: Some("ci".into()),
            prompt: "go".into(),
        };
        assert_eq!(h.headline(40).unwrap(), "♻ heartbeat ci · go");
    }
}
