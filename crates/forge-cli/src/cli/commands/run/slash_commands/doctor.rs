//! `/doctor`: `forge doctor`'s local checks without leaving the session. The live probes
//! (provider reachability, bridge round-trips) stay with the CLI: they make network calls and a
//! bridge round-trip spends subscription quota, which a habitual `/doctor` must not.

use forge_tui::{App, Deferred};

use crate::doctor::{Check, Status};

pub(super) fn start(app: &mut App) {
    app.note("⧖ running local checks…");
    let inbox = app.deferred_inbox();
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        let sections = collect(&runtime);
        let (text, summary) = render(&sections);
        inbox.post(Deferred::View {
            title: "/doctor".to_string(),
            summary,
            text,
            diff: false,
        });
    });
}

fn collect(runtime: &tokio::runtime::Handle) -> Vec<(&'static str, Vec<Check>)> {
    let mut sections = vec![
        ("Config", crate::doctor::config_checks()),
        ("Providers", crate::doctor::provider_checks().0),
        ("Model health", crate::doctor_health::model_health_checks()),
        (
            "Background daemon",
            runtime.block_on(crate::doctor::daemon_checks()),
        ),
        ("Forge Anywhere", crate::doctor_health::anywhere_checks()),
    ];
    sections.extend(crate::doctor_health::lsp_section());
    sections.push(("Local LLM (Ollama)", crate::doctor::ollama_checks()));
    sections.push(("Session store", crate::doctor::store_checks()));
    sections.push((
        "Environment",
        crate::doctor_environment::environment_checks_in_tui(),
    ));
    sections
}

/// The report text and its one-line verdict.
fn render(sections: &[(&str, Vec<Check>)]) -> (String, String) {
    let (mut fails, mut warns) = (0, 0);
    let mut text = String::new();
    for (title, checks) in sections {
        text.push_str(&format!("{title}\n"));
        for c in checks {
            text.push_str(&format!(
                "  {} {:<22} {}\n",
                c.status.glyph(),
                c.label,
                c.detail
            ));
            if matches!(c.status, Status::Warn | Status::Fail) {
                if let Some(fix) = &c.fix {
                    text.push_str(&format!("      → {fix}\n"));
                }
            }
            match c.status {
                Status::Fail => fails += 1,
                Status::Warn => warns += 1,
                Status::Ok | Status::Info => {}
            }
        }
        text.push('\n');
    }
    text.push_str("`forge doctor` in a terminal also probes providers and CLI bridges live.\n");
    let summary = if fails == 0 && warns == 0 {
        "all local checks pass".to_string()
    } else {
        format!("{fails} failure(s), {warns} warning(s)")
    };
    (text, summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::check;

    #[test]
    fn the_report_shows_fixes_only_for_problems_and_counts_them() {
        let sections = vec![(
            "Config",
            vec![
                check(Status::Ok, "config file", "parsed", Some("never shown")),
                check(
                    Status::Warn,
                    "unknown key",
                    "tui.colour",
                    Some("did you mean tui.color?"),
                ),
                check(Status::Fail, "store", "cannot open", Some("forge doctor")),
                check(Status::Info, "keys", "none", Some("not shown either")),
            ],
        )];
        let (text, summary) = render(&sections);
        assert!(text.starts_with("Config\n"));
        assert!(text.contains("→ did you mean tui.color?"));
        assert!(!text.contains("never shown") && !text.contains("not shown either"));
        assert_eq!(summary, "1 failure(s), 1 warning(s)");
        assert_eq!(render(&[("Config", vec![])]).1, "all local checks pass");
    }
}
