//! The TUI side of `/control`: a picker over the control tools. Enter switches the selected tool
//! on or off (the same switch the Settings pages and `forge mcp control` use); the newest audit
//! lines follow the tools so the menu also answers "what did the agent just do".

use forge_core::control;
use forge_tui::{PickerKind, PickerRow};

const AUDIT_ROWS: usize = 8;

pub(crate) fn rows(disabled: &[String], audit: &[serde_json::Value]) -> Vec<PickerRow> {
    let mut rows: Vec<PickerRow> = control::catalog(disabled)
        .into_iter()
        .map(|tool| PickerRow {
            id: tool.name.clone(),
            title: format!(
                "{} {}",
                if tool.enabled { "[on] " } else { "[off]" },
                tool.name
            ),
            subtitle: format!(
                "{}  {}",
                if tool.kind == "read" {
                    "reads"
                } else {
                    "acts "
                },
                tool.description
            ),
        })
        .collect();
    for entry in audit.iter().take(AUDIT_ROWS) {
        let route = entry["route"].as_str().unwrap_or("");
        rows.push(PickerRow {
            id: "audit".into(),
            title: format!(
                "recent  {} {}",
                route.trim_start_matches("control/"),
                entry["decision"].as_str().unwrap_or("")
            ),
            subtitle: entry["detail"].as_str().unwrap_or("").to_string(),
        });
    }
    rows
}

/// Open (or refresh) the menu with the cursor on `keep`, when that tool is still listed.
pub(crate) fn open(app: &mut forge_tui::App, keep: Option<&str>) {
    let rows = rows(
        &crate::control_settings::disabled(),
        &crate::serve_control::recent_audit(AUDIT_ROWS),
    );
    let at = keep.and_then(|id| rows.iter().position(|r| r.id == id));
    app.picker.open_with(
        PickerKind::ControlTools,
        "control tools - what agents may do to this app",
        rows,
    );
    if let Some(at) = at {
        app.picker.selected = at;
    }
}

pub(crate) fn accept(app: &mut forge_tui::App, row: &PickerRow) {
    if row.id == "audit" {
        open(app, None);
        return;
    }
    let now_enabled = crate::control_settings::disabled().contains(&row.id);
    match crate::control_settings::set_enabled(&row.id, now_enabled) {
        Ok(_) => app.note(&format!(
            "control: {} {}",
            row.id,
            if now_enabled { "on" } else { "off" }
        )),
        Err(error) => app.note(&format!("control: {error}")),
    }
    open(app, Some(&row.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_come_first_with_their_state_then_the_recent_calls() {
        let audit = vec![serde_json::json!({
            "route": "control/create_session",
            "decision": "allow",
            "detail": "{} -> created session abc"
        })];
        let rows = rows(&["merge_worktree".to_string()], &audit);
        let merge = rows.iter().find(|r| r.id == "merge_worktree").unwrap();
        assert!(merge.title.starts_with("[off]"));
        let usage = rows.iter().find(|r| r.id == "get_usage").unwrap();
        assert!(usage.title.starts_with("[on]"));
        assert!(usage.subtitle.starts_with("reads"));
        let last = rows.last().unwrap();
        assert_eq!(last.id, "audit");
        assert!(last.title.contains("create_session allow"));
    }
}
