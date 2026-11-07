//! Client side of `GET /api/brief`: fetching it from the local daemon and putting it in words.
//! Shared by `forge brief` and the board's "while you were away" strip.

use anyhow::{bail, Result};

use crate::serve_brief::{Brief, BriefSession, Outcome};

pub(crate) async fn fetch_brief(base: &str, token: &str, since: i64) -> Result<Brief> {
    let http = crate::attach::local_http_client(base);
    let resp = http
        .get(format!("{base}/{token}/api/brief"))
        .query(&[("since", since)])
        .send()
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "could not reach the forge serve daemon at {base} — is it running? \
                 (start it with `forge serve --local`)  [{e}]"
            )
        })?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        bail!("the daemon answered 404 — the token is wrong or the daemon predates `forge brief` (restart `forge serve`)");
    }
    if !status.is_success() {
        bail!("the daemon refused the request ({status})");
    }
    Ok(resp.json().await?)
}

/// `90s`, `30m`, `8h`, `2d`; a bare number is hours.
pub(crate) fn parse_window(text: &str) -> Result<i64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().find(|(_, c)| !c.is_ascii_digit()) {
        Some((at, _)) => text.split_at(at),
        None => (text, "h"),
    };
    let n: i64 = digits
        .parse()
        .map_err(|_| anyhow::anyhow!("--since takes a window like 90m, 8h or 2d (got {text:?})"))?;
    let secs = match unit.trim() {
        "s" => 1,
        "m" => 60,
        "h" | "" => 3600,
        "d" => 86400,
        other => bail!("unknown time unit {other:?} in --since (use s, m, h or d)"),
    };
    Ok(n.saturating_mul(secs))
}

pub(crate) fn span(secs: i64) -> String {
    let secs = secs.max(0);
    match secs {
        0..=89 => "moments".to_string(),
        90..=5399 => format!("{} min", (secs + 30) / 60),
        5400..=172_799 => format!("{}h", (secs + 1800) / 3600),
        _ => format!("{}d", (secs + 43_200) / 86_400),
    }
}

pub(crate) fn money(usd: f64) -> String {
    format!("${usd:.2}")
}

/// "3 finished, 1 failed, 1 waiting on you, 2 have changes ready to land, $3.10 spent".
pub(crate) fn headline(brief: &Brief) -> String {
    let t = &brief.totals;
    let mut parts = Vec::new();
    for (n, label) in [
        (t.finished, "finished"),
        (t.failed, "failed"),
        (t.waiting, "waiting on you"),
        (t.interrupted, "interrupted"),
        (t.working, "still working"),
    ] {
        if n > 0 {
            parts.push(format!("{n} {label}"));
        }
    }
    if t.changes_ready > 0 {
        parts.push(format!("{} with changes ready to land", t.changes_ready));
    }
    if t.spend_usd >= 0.005 {
        parts.push(format!("{} spent", money(t.spend_usd)));
    }
    parts.join(", ")
}

pub(crate) fn title_line(brief: &Brief) -> String {
    format!("While you were away ({})", span(brief.now - brief.since))
}

pub(crate) fn session_line(s: &BriefSession, now: i64) -> String {
    let state = match s.outcome {
        Outcome::Waiting => format!(
            "waiting on you for {}",
            span(now - s.waiting_since.unwrap_or(s.last_activity))
        ),
        Outcome::Failed => format!("failed: {}", s.reason.as_deref().unwrap_or("turn failed")),
        Outcome::Interrupted => "interrupted by a restart".to_string(),
        Outcome::Working => "still working".to_string(),
        Outcome::Finished => "finished".to_string(),
    };
    let mut line = format!("{} [{}] — {state}", s.title, s.project);
    if let Some(u) = &s.unlanded {
        line.push_str(&format!(" · +{}/-{} to land", u.added, u.removed));
    }
    if s.spend_usd >= 0.005 {
        line.push_str(&format!(" · {}", money(s.spend_usd)));
    }
    line
}

pub(crate) fn render(brief: &Brief) -> String {
    let mut drain = brief
        .drain
        .as_ref()
        .map_or_else(String::new, |d| format!("  {}\n", d.line));
    if let Some(q) = &brief.queue {
        drain.push_str(&format!("  {}\n", q.line));
    }
    if brief.sessions.is_empty() {
        if drain.is_empty() {
            return format!("{}: nothing happened.", title_line(brief));
        }
        return format!("{}: nothing finished.\n{drain}", title_line(brief));
    }
    let mut out = format!("{}: {}\n{drain}", title_line(brief), headline(brief));
    for s in &brief.sessions {
        out.push_str(&format!("  {}\n", session_line(s, brief.now)));
        if s.outcome != Outcome::Failed && !s.summary.is_empty() {
            out.push_str(&format!("      {}\n", s.summary));
        }
        out.push_str(&format!("      {}\n", s.id));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serve_brief::{BriefTotals, Unlanded};

    fn session(id: &str, outcome: Outcome) -> BriefSession {
        BriefSession {
            id: id.into(),
            title: format!("t-{id}"),
            cwd: "/w/app".into(),
            project: "app".into(),
            outcome,
            reason: None,
            waiting_since: None,
            last_activity: 1000,
            summary: "did a thing".into(),
            spend_usd: 1.25,
            worktree: false,
            unlanded: None,
        }
    }

    #[test]
    fn windows_parse_with_units_and_reject_junk() {
        assert_eq!(parse_window("8h").unwrap(), 28_800);
        assert_eq!(parse_window("30m").unwrap(), 1800);
        assert_eq!(parse_window("2d").unwrap(), 172_800);
        assert_eq!(parse_window("3").unwrap(), 10_800);
        assert!(parse_window("soon").is_err());
        assert!(parse_window("5w").is_err());
    }

    #[test]
    fn spans_read_naturally() {
        assert_eq!(span(10), "moments");
        assert_eq!(span(40 * 60), "40 min");
        assert_eq!(span(6 * 3600), "6h");
        assert_eq!(span(3 * 86_400), "3d");
    }

    #[test]
    fn headline_names_only_the_buckets_that_exist() {
        let brief = Brief {
            since: 0,
            now: 6 * 3600,
            totals: BriefTotals {
                sessions: 4,
                finished: 3,
                failed: 1,
                changes_ready: 2,
                spend_usd: 3.1,
                ..BriefTotals::default()
            },
            sessions: Vec::new(),
            drain: None,
            queue: None,
        };
        assert_eq!(
            headline(&brief),
            "3 finished, 1 failed, 2 with changes ready to land, $3.10 spent"
        );
        assert_eq!(title_line(&brief), "While you were away (6h)");
    }

    #[test]
    fn a_waiting_line_says_for_how_long_and_unlanded_lines_show() {
        let mut s = session("a", Outcome::Waiting);
        s.waiting_since = Some(1000);
        s.unlanded = Some(Unlanded {
            added: 4,
            removed: 2,
        });
        let line = session_line(&s, 1000 + 40 * 60);
        assert!(line.contains("waiting on you for 40 min"), "{line}");
        assert!(line.contains("+4/-2 to land"), "{line}");
        assert!(line.contains("$1.25"), "{line}");
    }

    #[test]
    fn a_drain_adds_its_line_even_when_no_session_finished() {
        let brief = Brief {
            since: 0,
            now: 3600,
            totals: BriefTotals::default(),
            sessions: Vec::new(),
            drain: Some(crate::serve_brief::DrainBrief {
                id: "d1".into(),
                line: "Drain: 7 started, 4 merged, 2 waiting on you, 1 blocked".into(),
                status: "active".into(),
                queued: 0,
            }),
            queue: None,
        };
        let out = render(&brief);
        assert!(out.contains("Drain: 7 started, 4 merged, 2 waiting on you, 1 blocked"));
        assert!(!out.contains("nothing happened"));
    }
}
