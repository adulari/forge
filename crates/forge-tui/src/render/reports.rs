//! Report renderers: the MCP server listing and the assay report, as scrollback lines.

use crate::theme::{color, styles};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::INDENT;

/// Color for a finding's severity (most→least): critical=red, high=orange, medium=yellow, low=dim.
fn severity_color(sev: forge_types::Severity) -> Color {
    use forge_types::Severity;
    match sev {
        Severity::Critical => color::err(),
        Severity::High => color::brand(),
        Severity::Medium => color::warn(),
        Severity::Low => color::fg_muted(),
    }
}

/// Render the MCP server listing (`/mcp`) as styled scrollback lines: a status glyph per server,
/// its transport, and tool/resource/prompt counts (mcp-client.md §5.7).
pub fn mcp_status_lines(servers: &[forge_types::McpServerLine]) -> Vec<Line<'static>> {
    if servers.is_empty() {
        let project = std::path::Path::new(".forge/mcp.toml");
        if let Some(problem) = forge_config::mcp_toml_problem(project) {
            return vec![Line::from(Span::styled(
                format!(
                    "  {} .forge/mcp.toml is ignored: {problem}",
                    crate::theme::glyphs::g().warn
                ),
                styles::warn(),
            ))];
        }
        return vec![Line::from(Span::styled(
            "  no MCP servers configured — declare them in .forge/mcp.toml (or `forge mcp import`)"
                .to_string(),
            styles::muted(),
        ))];
    }
    let mut out = vec![Line::from(Span::styled(
        format!("  MCP servers  ({} configured)", servers.len()),
        styles::user().add_modifier(Modifier::BOLD),
    ))];
    for s in servers {
        let (glyph, color) = match s.status.as_str() {
            "connected" => ("●", color::ok()),
            "slow" => ("●", color::warn()),
            "reconnecting" => ("↻", color::warn()),
            "failed" | "unauthorized" => ("○", color::err()),
            _ => ("○", color::fg_muted()), // disabled / unknown
        };
        let mut spans = vec![
            Span::styled(format!("    {glyph} "), Style::default().fg(color)),
            Span::styled(
                format!("{:<12}", s.name),
                styles::fg().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<13}{:<6}", s.status, s.transport),
                Style::default().fg(color),
            ),
            Span::styled(
                format!(
                    "{} tools · {} resources · {} prompts",
                    s.tools, s.resources, s.prompts
                ),
                styles::muted(),
            ),
        ];
        if let Some(detail) = &s.detail {
            spans.push(Span::styled(format!("   {detail}"), styles::err()));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        "    tools load on demand — the model finds them with tool_search or mcp_search_tools."
            .to_string(),
        styles::muted(),
    )));
    out
}

/// Render an [`AssayReport`](forge_types::AssayReport) as styled lines for the TUI scrollback:
/// a colored summary header, then each ranked finding with severity-colored gutter, location,
/// title, why, and suggested fix.
pub fn assay_report_lines(r: &forge_types::AssayReport) -> Vec<Line<'static>> {
    let [crit, high, med, low] = r.severity_counts();
    let id8: String = r.run_id.chars().take(8).collect();
    let mut out = vec![
        Line::from(Span::styled(
            format!(
                "{INDENT}ASSAY REPORT  run {id8}  scope: {}",
                r.scope.label()
            ),
            styles::brand().add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                format!("{INDENT}{} findings  ", r.findings.len()),
                styles::fg(),
            ),
            Span::styled(format!("{crit} crit "), styles::err()),
            Span::styled(format!("{high} high "), styles::brand()),
            Span::styled(format!("{med} med "), styles::warn()),
            Span::styled(format!("{low} low"), styles::muted()),
            Span::styled(format!("  ·  ${:.4}", r.cost_usd), styles::ok()),
        ]),
    ];
    if !r.skipped_lenses.is_empty() {
        let sk: Vec<String> = r
            .skipped_lenses
            .iter()
            .map(|(l, why)| format!("{l} ({why})"))
            .collect();
        out.push(Line::from(Span::styled(
            format!("{INDENT}skipped: {}", sk.join(", ")),
            styles::muted(),
        )));
    }
    if r.findings.is_empty() {
        out.push(Line::from(Span::styled(
            format!("{INDENT}no findings — clean, or the scope had no analyzable source."),
            styles::muted(),
        )));
        return out;
    }
    for (i, f) in r.findings.iter().enumerate() {
        let color = severity_color(f.severity);
        let loc = match f.line {
            Some(l) => format!("{}:{l}", f.file),
            None => f.file.clone(),
        };
        out.push(Line::from(vec![
            Span::styled(format!("{INDENT}{:>2}. ", i + 1), styles::muted()),
            Span::styled(
                format!("{} ", f.severity.as_str().to_uppercase()),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{} ", f.category.as_str()), styles::fg()),
            Span::styled(loc, styles::muted()),
        ]));
        out.push(Line::from(Span::styled(
            format!("{INDENT}    {}", f.title),
            styles::fg(),
        )));
        if !f.suggested_fix.is_empty() {
            out.push(Line::from(Span::styled(
                format!(
                    "{INDENT}    fix: {} ({})",
                    f.suggested_fix,
                    f.effort.as_str()
                ),
                styles::muted(),
            )));
        }
    }
    out
}

/// Plain (no-ANSI) rendering of an assay report, for the headless/piped presenter.
pub fn assay_report_plain(r: &forge_types::AssayReport) -> String {
    let [crit, high, med, low] = r.severity_counts();
    let id8: String = r.run_id.chars().take(8).collect();
    let mut s = format!("\nASSAY REPORT  run {id8}  scope: {}\n", r.scope.label());
    s.push_str(&format!(
        "{} findings · {crit} critical · {high} high · {med} medium · {low} low · ${:.4}\n",
        r.findings.len(),
        r.cost_usd
    ));
    if !r.skipped_lenses.is_empty() {
        let sk: Vec<String> = r
            .skipped_lenses
            .iter()
            .map(|(l, why)| format!("{l} ({why})"))
            .collect();
        s.push_str(&format!("skipped: {}\n", sk.join(", ")));
    }
    if r.findings.is_empty() {
        s.push_str("no findings — clean, or the scope had no analyzable source.\n");
        return s;
    }
    for (i, f) in r.findings.iter().enumerate() {
        let loc = match f.line {
            Some(l) => format!("{}:{l}", f.file),
            None => f.file.clone(),
        };
        s.push_str(&format!(
            "{:>2}. [{} · {}] {} — {loc}\n    {}\n",
            i + 1,
            f.severity.as_str().to_uppercase(),
            f.confidence.as_str(),
            f.category.as_str(),
            f.title,
        ));
        if !f.suggested_fix.is_empty() {
            s.push_str(&format!(
                "    fix: {} ({})\n",
                f.suggested_fix,
                f.effort.as_str()
            ));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_of(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn assay_report_renders_summary_and_ranked_findings() {
        use forge_types::{
            AssayReport, AssayScope, Confidence, Effort, Finding, FindingCategory, Severity,
        };
        let report = AssayReport {
            run_id: "abcdef123456".into(),
            scope: AssayScope::Repo,
            findings: vec![Finding {
                id: "f1".into(),
                category: FindingCategory::Correctness,
                severity: Severity::Critical,
                confidence: Confidence::High,
                file: "core/lib.rs".into(),
                line: Some(204),
                title: "unwrap panics the turn".into(),
                rationale: "a 5xx aborts the session".into(),
                suggested_fix: "propagate via ?".into(),
                effort: Effort::Small,
                lens: "correctness".into(),
                verified: true,
            }],
            cost_usd: 0.118,
            skipped_lenses: vec![("design".into(), "timeout".into())],
        };
        let t = text_of(&assay_report_lines(&report));
        assert!(t.contains("run abcdef12"), "short run id: {t}");
        assert!(t.contains("1 findings"), "summary count");
        assert!(t.contains("skipped: design (timeout)"), "degradation noted");
        assert!(t.contains("CRITICAL") && t.contains("core/lib.rs:204"));
        assert!(t.contains("fix: propagate via ?"));

        // Plain form is ANSI-free for pipes.
        let plain = assay_report_plain(&report);
        assert!(!plain.contains('\u{1b}'), "no ANSI in plain report");
        assert!(plain.contains("CRITICAL"));
    }
}
