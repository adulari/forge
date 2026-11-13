//! `forge ship <session>`: commit the session's changes, push its branch, open the pull request
//! and arm PR Watch in merge mode, through the daemon's `POST /api/sessions/{id}/ship`. Prints one
//! line per step and exits non-zero at the first refusal, naming the step and why.

use anyhow::{bail, Result};
use forge_tui::ship_menu::ShipOutcome;

use crate::attach::resolve_session_id;
use crate::cli::commands::issues::Daemon;

pub(crate) fn request_body(
    title: Option<&str>,
    message: Option<&str>,
    base: Option<&str>,
    draft: bool,
    method: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    for (key, value) in [
        ("title", title),
        ("message", message),
        ("base", base),
        ("method", method),
    ] {
        if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
            body.insert(key.into(), v.into());
        }
    }
    if draft {
        body.insert("draft".into(), true.into());
    }
    serde_json::Value::Object(body)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn ship_cmd(
    session: String,
    title: Option<String>,
    message: Option<String>,
    base: Option<String>,
    draft: bool,
    method: Option<String>,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let daemon = Daemon::resolve(url, token)?;
    let running = daemon.running_sessions().await?;
    let id = resolve_session_id(&running, &session)?;
    let body = request_body(
        title.as_deref(),
        message.as_deref(),
        base.as_deref(),
        draft,
        method.as_deref(),
    );
    let answer = daemon.ship(&id, &body).await?;
    let outcome = ShipOutcome::from_json(&answer);
    if let Some(plan) = &outcome.plan {
        for line in plan.lines(50) {
            println!("{line}");
        }
    }
    for line in outcome.summary_lines() {
        println!("{line}");
    }
    if let Some(url) = outcome.pr_url.as_deref().filter(|u| !u.is_empty()) {
        println!("{url}");
    }
    if !outcome.ok {
        bail!("not shipped");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_carries_only_what_was_given() {
        assert_eq!(
            request_body(None, None, None, false, None),
            serde_json::json!({})
        );
        assert_eq!(
            request_body(
                Some(" Add cart "),
                Some(""),
                Some("main"),
                true,
                Some("rebase")
            ),
            serde_json::json!({"title": "Add cart", "base": "main", "draft": true, "method": "rebase"})
        );
    }
}
