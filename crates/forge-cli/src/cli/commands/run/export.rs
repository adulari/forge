//! `/export [path]` — render the CURRENT session's transcript to one self-contained HTML file
//! (docs/features/session-replay.md, "Shipped (follow-up 3)"). Shares its renderer with
//! `forge replay <id> --html <path>` (`crate::replay::render_html`). Lives in its own file
//! because `run/dispatch.rs` sits at its CI file-size ratchet ceiling — the dispatch arm just
//! forwards here. The export is always secret-redacted (`forge_core::redact`).

use super::*;

/// Default export path when `/export` is called with no argument: `forge-session-<id8>.html` in
/// the current directory, mirroring `forge replay`'s 8-char short id convention.
fn default_export_path(id: &str, json: bool) -> String {
    let ext = if json { "json" } else { "html" };
    format!("forge-session-{}.{ext}", &id[..id.len().min(8)])
}

/// `/export --json [path]`: `--json` may sit anywhere in the argument; what remains is the path.
fn split_json_flag(arg: Option<String>) -> (bool, Option<String>) {
    let Some(arg) = arg else {
        return (false, None);
    };
    let mut json = false;
    let rest: Vec<&str> = arg
        .split_whitespace()
        .filter(|token| {
            let flag = *token == "--json";
            json |= flag;
            !flag
        })
        .collect();
    (json, (!rest.is_empty()).then(|| rest.join(" ")))
}

pub(crate) async fn export_cmd(
    session: &Arc<tokio::sync::Mutex<Session>>,
    path_arg: Option<String>,
    app: &mut forge_tui::App,
) -> Result<DispatchOutcome> {
    let (id, mut entries) = {
        let s = session.lock().await;
        let id = s.session_id().to_string();
        let entries = s.load_replay(&id).map_err(|e| anyhow::anyhow!("{e}"))?;
        (id, entries)
    };
    if entries.is_empty() {
        app.note("● nothing to export yet — this session has no messages");
        return Ok(DispatchOutcome::Handled);
    }
    forge_core::redact::Redactor::from_environment().redact_replay(&mut entries);
    let short = &id[..id.len().min(8)];
    let (json, path_arg) = split_json_flag(path_arg);
    let path = path_arg.unwrap_or_else(|| default_export_path(&id, json));
    let body = if json {
        crate::replay::render_json(&id, &entries)
    } else {
        crate::replay::render_html(short, &entries)
    };
    if let Err(e) = std::fs::write(&path, body) {
        app.note(&format!("● export failed: could not write {path}: {e}"));
        return Ok(DispatchOutcome::Handled);
    }
    app.note(&format!("✓ exported session {short} to {path}"));
    Ok(DispatchOutcome::Handled)
}

#[cfg(test)]
mod tests {
    use super::split_json_flag;

    #[test]
    fn the_json_flag_is_split_from_the_path() {
        assert_eq!(split_json_flag(None), (false, None));
        assert_eq!(
            split_json_flag(Some("out.html".into())),
            (false, Some("out.html".into()))
        );
        assert_eq!(split_json_flag(Some("--json".into())), (true, None));
        assert_eq!(
            split_json_flag(Some("--json a/b.json".into())),
            (true, Some("a/b.json".into()))
        );
        assert_eq!(
            split_json_flag(Some("a b.json --json".into())),
            (true, Some("a b.json".into()))
        );
    }
}
