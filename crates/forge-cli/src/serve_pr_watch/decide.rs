//! PR Watch's pure decision step: what a polled PR summary means for the watch.

use super::*;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Wait(Option<String>),
    Block(String),
    Fix { key: String, prompt: String },
    Merge,
    Merged,
    Green,
}

fn failing_lines(pr: &PrSummary) -> Vec<String> {
    let mut lines: Vec<String> = pr
        .checks
        .items
        .iter()
        .filter(|c| c.bucket == "failing")
        .map(|c| match &c.url {
            Some(url) => format!("- {} ({}): {url}", c.name, c.detail),
            None => format!("- {} ({})", c.name, c.detail),
        })
        .collect();
    lines.sort();
    lines
}

fn fix_prompt(pr: &PrSummary, failing: &[String], changes: bool, new_comments: usize) -> String {
    let mut text = format!(
        "PR Watch: pull request #{} ({}) needs attention.\n",
        pr.number, pr.url
    );
    if !failing.is_empty() {
        text.push_str("\nFailing checks:\n");
        text.push_str(&failing.join("\n"));
        text.push('\n');
    }
    if changes {
        text.push_str("\nA reviewer requested changes.\n");
    }
    if new_comments > 0 {
        text.push_str(&format!(
            "\n{new_comments} new comment(s) arrived; read them with `gh pr view {} --comments`.\n",
            pr.number
        ));
    }
    text.push_str(
        "\nFix the cause in this worktree, run the relevant checks locally (when the repository has \
         workflows, `forge ci run` runs the same steps CI does, with CI's environment), commit, \
         and push with a normal `git push`. Never force-push, never rewrite history, never delete the \
         branch. If you cannot fix it, say why and stop.",
    );
    text
}

pub(super) fn decide(watch: &Watch, pr: &PrSummary, comments: &[String], free: bool) -> Decision {
    match pr.state.as_str() {
        "merged" => return Decision::Merged,
        "closed" => return Decision::Block("the PR was closed without merging".into()),
        _ => {}
    }
    if pr.draft {
        return Decision::Wait(Some("the PR is a draft".into()));
    }
    let conflicting = pr.mergeable == "conflicting" || pr.merge_state == "dirty";
    if watch.mode == Mode::Merge && watch.method == "auto" && !watch.auto_armed && !conflicting {
        if let Some(reason) = &watch.hold {
            return Decision::Wait(Some(reason.clone()));
        }
        return Decision::Merge;
    }
    if pr.checks.pending > 0 {
        return Decision::Wait(None);
    }
    let failing = pr.checks.failing > 0;
    let changes = pr.review_decision == "changes_requested";
    let fresh: Vec<&String> = comments
        .iter()
        .filter(|id| !watch.seen_comments.contains(id))
        .collect();
    if failing || changes || !fresh.is_empty() {
        let lines = failing_lines(pr);
        let mut ids: Vec<&str> = fresh.iter().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        let key = format!(
            "{}|{}|{changes}|{}",
            pr.head_sha,
            lines.join(";"),
            ids.join(",")
        );
        if watch.last_key.as_deref() == Some(key.as_str()) {
            return Decision::Wait(Some("fix turn sent; waiting for a push".into()));
        }
        if !free {
            return Decision::Wait(None);
        }
        if watch.fix_turns >= MAX_FIX_TURNS {
            return Decision::Block(format!(
                "gave up after {MAX_FIX_TURNS} fix turns without a green PR"
            ));
        }
        return Decision::Fix {
            prompt: fix_prompt(pr, &lines, changes, fresh.len()),
            key,
        };
    }
    if watch.mode != Mode::Merge {
        return Decision::Green;
    }
    if let Some(reason) = &watch.hold {
        return Decision::Wait(Some(reason.clone()));
    }
    if conflicting {
        return Decision::Wait(Some("merge conflicts with the base branch".into()));
    }
    match pr.merge_state.as_str() {
        "clean" | "has_hooks" | "unstable" => Decision::Merge,
        "behind" => Decision::Wait(Some("the branch is behind its base".into())),
        "blocked" => Decision::Wait(Some("waiting for a required review or check".into())),
        other => Decision::Wait(Some(format!("merge state is {other}"))),
    }
}

pub(super) fn comment_ids(value: &Value) -> Vec<String> {
    ["comments", "reviews"]
        .iter()
        .flat_map(|key| {
            value
                .get(*key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}
