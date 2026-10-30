//! `gh pr view --json` rows reduced to what the app shows.

use serde::Serialize;
use serde_json::Value;

pub(crate) const SUMMARY_FIELDS: &str = "number,url,title,state,isDraft,headRefName,baseRefName,headRefOid,mergeable,mergeStateStatus,reviewDecision,statusCheckRollup,additions,deletions,changedFiles";

#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct CheckItem {
    pub(crate) name: String,
    /// `pending` | `passing` | `failing`
    pub(crate) bucket: &'static str,
    pub(crate) detail: String,
    pub(crate) url: Option<String>,
    pub(crate) workflow: Option<String>,
}

#[derive(Serialize, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChecksSummary {
    pub(crate) total: usize,
    pub(crate) passing: usize,
    pub(crate) failing: usize,
    pub(crate) pending: usize,
    pub(crate) items: Vec<CheckItem>,
}

pub(crate) fn summarize_checks(rollup: &Value) -> ChecksSummary {
    let mut summary = ChecksSummary::default();
    let text = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    for item in rollup.as_array().into_iter().flatten() {
        let (name, bucket, detail, url) = if item.get("context").is_some() {
            let state = text(item, "state").unwrap_or_default();
            let bucket = match state.as_str() {
                "SUCCESS" => "passing",
                "PENDING" | "EXPECTED" => "pending",
                _ => "failing",
            };
            (
                text(item, "context").unwrap_or_default(),
                bucket,
                state.to_lowercase(),
                text(item, "targetUrl"),
            )
        } else {
            let status = text(item, "status").unwrap_or_default();
            let conclusion = text(item, "conclusion").unwrap_or_default();
            let bucket = if status != "COMPLETED" {
                "pending"
            } else if matches!(conclusion.as_str(), "SUCCESS" | "NEUTRAL" | "SKIPPED") {
                "passing"
            } else {
                "failing"
            };
            let detail = if status == "COMPLETED" {
                conclusion.to_lowercase()
            } else {
                status.to_lowercase().replace('_', " ")
            };
            (
                text(item, "name").unwrap_or_default(),
                bucket,
                detail,
                text(item, "detailsUrl"),
            )
        };
        match bucket {
            "passing" => summary.passing += 1,
            "failing" => summary.failing += 1,
            _ => summary.pending += 1,
        }
        summary.total += 1;
        summary.items.push(CheckItem {
            name,
            bucket,
            detail,
            url,
            workflow: text(item, "workflowName"),
        });
    }
    summary
}

#[derive(Serialize, Debug)]
pub(crate) struct PrSummary {
    pub(crate) number: u64,
    pub(crate) url: String,
    pub(crate) title: String,
    /// `open` | `closed` | `merged`
    pub(crate) state: String,
    pub(crate) draft: bool,
    pub(crate) head_ref: String,
    pub(crate) base_ref: String,
    pub(crate) head_sha: String,
    pub(crate) mergeable: String,
    pub(crate) merge_state: String,
    pub(crate) review_decision: String,
    pub(crate) additions: u64,
    pub(crate) deletions: u64,
    pub(crate) changed_files: u64,
    pub(crate) checks: ChecksSummary,
}

pub(crate) fn parse_summary(value: &Value) -> Option<PrSummary> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let count = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    Some(PrSummary {
        number: value.get("number")?.as_u64()?,
        url: text("url"),
        title: text("title"),
        state: text("state").to_lowercase(),
        draft: value
            .get("isDraft")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        head_ref: text("headRefName"),
        base_ref: text("baseRefName"),
        head_sha: text("headRefOid"),
        mergeable: text("mergeable").to_lowercase(),
        merge_state: text("mergeStateStatus").to_lowercase(),
        review_decision: text("reviewDecision").to_lowercase(),
        additions: count("additions"),
        deletions: count("deletions"),
        changed_files: count("changedFiles"),
        checks: summarize_checks(value.get("statusCheckRollup").unwrap_or(&Value::Null)),
    })
}
