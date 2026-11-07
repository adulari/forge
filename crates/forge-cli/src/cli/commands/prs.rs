//! `forge prs queue`: the daemon's merge queue (`GET /api/pr/queue`), one block per repository.

use anyhow::Result;
use serde_json::Value;

use crate::cli::args::PrsOp;
use crate::cli::commands::issues::Daemon;

pub(crate) async fn prs_cmd(op: PrsOp) -> Result<()> {
    match op {
        PrsOp::Queue { json, url, token } => {
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
    fn an_empty_queue_says_how_to_fill_it() {
        assert!(queue_lines(&json!({"queues": []}))[0].contains("No pull requests"));
    }
}
