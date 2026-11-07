//! `forge prs queue`: the daemon's merge queue (`GET /api/pr/queue`), one block per repository.

use anyhow::Result;
use serde_json::Value;

use crate::cli::args::{PrsOp, QueueAction, QueueTarget};
use crate::cli::commands::issues::Daemon;

pub(crate) async fn prs_cmd(op: PrsOp) -> Result<()> {
    match op {
        PrsOp::Queue {
            action: Some(action),
            ..
        } => queue_action_cmd(action).await,
        PrsOp::Review(QueueTarget { pr, url, token }) => {
            let query = match pr.trim_start_matches('#').parse::<u64>() {
                Ok(n) => format!("pr={n}"),
                Err(_) => format!("session={pr}"),
            };
            let answer = Daemon::resolve(url, token)?.pr_review(&query).await?;
            for line in review_lines(&answer) {
                println!("{line}");
            }
            Ok(())
        }
        PrsOp::Queue {
            json, url, token, ..
        } => {
            let answer = Daemon::resolve(url, token)?.merge_queue().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&answer)?);
            } else {
                for line in queue_lines(&answer) {
                    println!("{line}");
                }
            }
            Ok(())
        }
    }
}

/// The body `POST /api/pr/queue/action` takes: a PR number, or a session id when it is not one.
pub(crate) fn action_body(target: &str, action: &str) -> Value {
    match target.trim_start_matches('#').parse::<u64>() {
        Ok(pr) => serde_json::json!({"pr": pr, "action": action}),
        Err(_) => serde_json::json!({"session": target, "action": action}),
    }
}

async fn queue_action_cmd(action: QueueAction) -> Result<()> {
    let (name, QueueTarget { pr, url, token }) = match action {
        QueueAction::Hold(t) => ("hold", t),
        QueueAction::Release(t) => ("release", t),
        QueueAction::Front(t) => ("front", t),
        QueueAction::Retry(t) => ("retry", t),
    };
    let answer = Daemon::resolve(url, token)?
        .queue_action(&action_body(&pr, name))
        .await?;
    println!("{}", answer["message"].as_str().unwrap_or("done"));
    for line in queue_lines(&answer) {
        println!("{line}");
    }
    Ok(())
}

/// The rows of `GET /api/pr/queue` (or an action's answer), repository by repository.
pub(crate) fn queue_rows(answer: &Value) -> Vec<forge_tui::queue_menu::QueueRow> {
    let text = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let mut out = Vec::new();
    for queue in answer["queues"].as_array().into_iter().flatten() {
        for row in queue["rows"].as_array().into_iter().flatten() {
            out.push(forge_tui::queue_menu::QueueRow {
                session_id: text(row, "session_id"),
                repo: text(queue, "repo"),
                pr: row["pr_number"].as_u64(),
                position: row["position"]
                    .as_u64()
                    .and_then(|p| usize::try_from(p).ok()),
                state: text(row, "state"),
                reason: text(row, "reason"),
                review: text(row, "review"),
                pushes: row["pushes"]
                    .as_u64()
                    .and_then(|p| u32::try_from(p).ok())
                    .unwrap_or(0),
            });
        }
    }
    out
}

/// `forge prs review <pr>`: the verdict, then each finding as `severity file:line title`.
pub(crate) fn review_lines(answer: &Value) -> Vec<String> {
    let pr = answer["pr_number"]
        .as_u64()
        .map_or_else(|| "PR ?".to_string(), |n| format!("PR #{n}"));
    let Some(review) = answer.get("review").filter(|r| !r.is_null()) else {
        return vec![format!("{pr}: not reviewed yet")];
    };
    let mut lines = vec![format!(
        "{pr}: {} (head {}, model {})",
        text(answer, "summary"),
        text(review, "head_sha").chars().take(7).collect::<String>(),
        review["model"].as_str().unwrap_or("-")
    )];
    if let Some(error) = review["error"].as_str() {
        lines.push(format!("  error: {error}"));
    }
    for f in review["findings"].as_array().into_iter().flatten() {
        let place = f["line"].as_u64().map_or_else(
            || text(f, "file").to_string(),
            |l| format!("{}:{l}", text(f, "file")),
        );
        let held = if f["blocking"].as_bool().unwrap_or(false) {
            "blocks"
        } else {
            "advisory"
        };
        lines.push(format!(
            "  [{}] {place} {} ({held})",
            text(f, "severity"),
            text(f, "title")
        ));
        lines.push(format!("      why: {}", text(f, "why")));
    }
    lines
}

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The table `forge prs queue` prints; empty queues say so.
pub(crate) fn queue_lines(answer: &Value) -> Vec<String> {
    let queues = answer["queues"].as_array().cloned().unwrap_or_default();
    if queues.is_empty() {
        return vec![
            "No pull requests are in a merge queue. Arm PR Watch in merge mode (`forge ship`, \
             `forge issues --drain`) to queue them."
                .into(),
        ];
    }
    let mut lines = Vec::new();
    for queue in &queues {
        lines.push(text(queue, "repo").to_string());
        for row in queue["rows"].as_array().into_iter().flatten() {
            let place = row["position"]
                .as_u64()
                .map_or_else(|| "-".to_string(), |p| format!("#{p}"));
            let pr = row["pr_number"]
                .as_u64()
                .map_or_else(|| "PR ?".to_string(), |n| format!("PR #{n}"));
            let mut line = format!("  {place:<3} {pr:<9} {:<8}", text(row, "state"));
            let reason = text(row, "reason");
            if !reason.is_empty() {
                line.push_str(&format!(" {reason}"));
            }
            if let Some(review) = row["review"].as_str() {
                line.push_str(&format!(" [{review}]"));
            }
            let pushes = row["pushes"].as_u64().unwrap_or(0);
            if pushes > 0 {
                line.push_str(&format!(
                    " ({pushes} push{})",
                    if pushes == 1 { "" } else { "es" }
                ));
            }
            lines.push(line);
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rows_show_position_state_reason_and_pushes() {
        let answer = json!({"queues": [{"repo": "/r/.git", "rows": [
            {"position": 1, "pr_number": 7, "state": "checks", "reason": "waiting for checks on the updated branch", "pushes": 1},
            {"position": 2, "pr_number": 9, "state": "queued", "reason": "#2 in the merge queue, behind PR #7", "pushes": 0},
            {"position": null, "pr_number": 8, "state": "skipped", "reason": "conflicts in src/a.rs", "pushes": 0},
        ]}]});
        let lines = queue_lines(&answer);
        assert_eq!(lines[0], "/r/.git");
        assert!(
            lines[1].starts_with("  #1  PR #7") && lines[1].ends_with("(1 push)"),
            "{}",
            lines[1]
        );
        assert!(lines[2].contains("queued") && lines[2].contains("behind PR #7"));
        assert!(lines[3].starts_with("  -   PR #8") && lines[3].contains("conflicts in src/a.rs"));
    }

    #[test]
    fn the_daemons_queue_becomes_rows_in_order_with_their_repository() {
        let rows = queue_rows(&json!({"queues": [
            {"repo": "r1 @ main", "rows": [
                {"session_id": "a", "pr_number": 7, "position": 1, "state": "head", "reason": null, "pushes": 2},
                {"session_id": "b", "pr_number": null, "position": null, "state": "skipped", "reason": "conflicts in x", "pushes": 0}]},
            {"repo": "r2 @ main", "rows": [
                {"session_id": "c", "pr_number": 7, "position": 1, "state": "queued", "reason": "x", "pushes": 0}]}]}));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0].pr, rows[0].position, rows[0].pushes),
            (Some(7), Some(1), 2)
        );
        assert_eq!(rows[0].reason, "");
        assert_eq!((rows[1].pr, rows[1].position), (None, None));
        assert_eq!(rows[2].repo, "r2 @ main");
        assert!(queue_rows(&json!({})).is_empty());
    }

    #[test]
    fn the_review_shows_in_the_queue_table_and_in_its_own_report() {
        let answer = json!({"queues": [{"repo": "r", "rows": [
            {"position": 1, "pr_number": 7, "state": "head", "reason": "", "pushes": 0, "review": "review: 2 confirmed, fixing"}]}]});
        assert!(queue_lines(&answer)[1].contains("[review: 2 confirmed, fixing]"));
        assert_eq!(queue_rows(&answer)[0].review, "review: 2 confirmed, fixing");
        let lines = review_lines(
            &json!({"pr_number": 7, "summary": "review: 1 confirmed, fixing",
            "review": {"head_sha": "abcdef123", "model": "m", "error": null, "findings": [
                {"severity": "high", "file": "src/a.rs", "line": 12, "title": "unwrap", "why": "panics", "blocking": true}]}}),
        );
        assert!(lines[0].starts_with("PR #7: review: 1 confirmed, fixing (head abcdef1, model m)"));
        assert_eq!(lines[1], "  [high] src/a.rs:12 unwrap (blocks)");
        assert_eq!(lines[2], "      why: panics");
        assert_eq!(
            review_lines(&json!({"pr_number": 7, "review": null})),
            ["PR #7: not reviewed yet"]
        );
    }

    #[test]
    fn actions_name_a_pr_number_or_a_session() {
        assert_eq!(
            action_body("12", "hold"),
            json!({"pr": 12, "action": "hold"})
        );
        assert_eq!(
            action_body("#12", "front"),
            json!({"pr": 12, "action": "front"})
        );
        assert_eq!(
            action_body("s_ab12", "retry"),
            json!({"session": "s_ab12", "action": "retry"})
        );
    }

    #[test]
    fn an_empty_queue_says_how_to_fill_it() {
        assert!(queue_lines(&json!({"queues": []}))[0].contains("No pull requests"));
    }
}
