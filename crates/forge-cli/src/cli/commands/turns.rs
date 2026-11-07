//! `forge turns`: how turns ended (docs/features/session-replay.md, "Turn endings"). Counts by end
//! reason, the last turns that did not finish, and `--backfill` for history recorded before the
//! `turn` table existed.

use crate::*;
use anyhow::{Context, Result};
use forge_store::TurnRow;

pub(crate) struct TurnsArgs {
    pub since: String,
    pub session: Option<String>,
    pub limit: usize,
    pub json: bool,
    pub backfill: bool,
    pub write: bool,
}

/// `7d`, `12h`, `30m` or a bare number of days, in seconds.
pub(crate) fn parse_window(text: &str) -> Result<i64> {
    let text = text.trim();
    let (digits, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&text[..i], c),
        _ => (text, 'd'),
    };
    let n: i64 = digits
        .parse()
        .with_context(|| format!("not a window like 7d, 12h or 30m: {text}"))?;
    let unit_secs = match unit {
        'd' => 86_400,
        'h' => 3_600,
        'm' => 60,
        other => anyhow::bail!("unknown window unit '{other}' (use d, h or m)"),
    };
    Ok(n * unit_secs)
}

pub(crate) fn turns_cmd(args: TurnsArgs) -> Result<()> {
    let store = open_store()?;
    let session = match args.session.as_deref() {
        None => None,
        Some(prefix) => Some(resolve_session(&store, prefix)?),
    };
    let now = chrono::Utc::now().timestamp();
    let since = now - parse_window(&args.since)?;
    if args.backfill {
        let turns = store
            .backfill_turns(session.as_deref(), since)
            .context("classifying history")?;
        if args.write {
            let written = store.write_backfilled_turns(&turns)?;
            println!("Stored {written} turn(s) classified from the transcript.");
        } else if args.json {
            println!("{}", serde_json::to_string_pretty(&turns)?);
        } else {
            print!("{}", render_backfill(&turns));
        }
        return Ok(());
    }
    let counts = store.turn_end_counts(since, session.as_deref())?;
    let unfinished = store.recent_unfinished_turns(since, session.as_deref(), args.limit)?;
    if args.json {
        let counts: serde_json::Map<String, serde_json::Value> = counts
            .iter()
            .map(|(reason, n)| (reason.clone(), (*n).into()))
            .collect();
        let value = serde_json::json!({
            "since": since,
            "counts": counts,
            "unfinished": unfinished,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print!("{}", render(&counts, &unfinished, &args.since));
    }
    Ok(())
}

fn when(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

pub(crate) fn render(counts: &[(String, i64)], unfinished: &[TurnRow], since: &str) -> String {
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    if total == 0 {
        return format!("No turns recorded in the last {since}.\n");
    }
    let mut out = format!("Turn endings, last {since} ({total} turns)\n");
    for (reason, n) in counts {
        out.push_str(&format!("  {reason:<28} {n:>6}\n"));
    }
    if !unfinished.is_empty() {
        out.push_str(&format!(
            "\nLast {} turns that did not finish\n  {:<7} {:<12} {:<26} {:<24} {}\n",
            unfinished.len(),
            "id",
            "time",
            "reason",
            "shape",
            "session"
        ));
        for t in unfinished {
            out.push_str(&format!(
                "  {:<7} {:<12} {:<26} {:<24} {}\n",
                t.id,
                when(t.started_at),
                t.end_reason.as_deref().unwrap_or("running"),
                t.last_role_shape.as_deref().unwrap_or("-"),
                &t.session_id[..t.session_id.len().min(8)]
            ));
        }
    }
    out
}

fn render_backfill(turns: &[TurnRow]) -> String {
    let mut by_reason: std::collections::BTreeMap<&str, i64> = Default::default();
    for t in turns {
        *by_reason
            .entry(t.end_reason.as_deref().unwrap_or("?"))
            .or_default() += 1;
    }
    let mut out = format!(
        "{} historical turn(s) not yet recorded (shape only; nothing written, add --write)\n",
        turns.len()
    );
    for (reason, n) in by_reason {
        out.push_str(&format!("  {reason:<28} {n:>6}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_parse_in_days_hours_and_minutes() {
        assert_eq!(parse_window("7d").unwrap(), 7 * 86_400);
        assert_eq!(parse_window("12h").unwrap(), 12 * 3_600);
        assert_eq!(parse_window("30m").unwrap(), 1_800);
        assert_eq!(parse_window("2").unwrap(), 2 * 86_400);
        assert!(parse_window("soon").is_err());
        assert!(parse_window("5y").is_err());
    }

    #[test]
    fn the_report_lists_counts_and_unfinished_turns() {
        let row = TurnRow {
            id: 9,
            session_id: "abcdef1234567890".into(),
            prompt_seq: 3,
            started_at: 1_700_000_000,
            ended_at: Some(1_700_000_050),
            end_reason: Some("stalled".into()),
            detail_json: None,
            last_role_shape: Some("unanswered_tool_results".into()),
        };
        let text = render(
            &[("final_answer".into(), 5), ("stalled".into(), 1)],
            &[row],
            "7d",
        );
        assert!(text.contains("(6 turns)"), "{text}");
        assert!(text.contains("final_answer"));
        assert!(text.contains("unanswered_tool_results"));
        assert!(text.contains("abcdef12"));
        assert_eq!(
            render(&[], &[], "7d"),
            "No turns recorded in the last 7d.\n"
        );
    }
}
