//! `forge stats codemode` — how often direct models use `run_code`, how often the script fails,
//! and what the programs cost in output tokens. docs/features/codemode.md ("Metrics").

pub(super) fn render(
    stats: &forge_store::CodemodeStats,
    session: Option<&str>,
    days: u32,
) -> String {
    let scope = match session {
        Some(id) => format!("session {}", id.chars().take(8).collect::<String>()),
        None => format!("last {days} days"),
    };
    if stats.runs == 0 {
        return format!("no run_code calls recorded ({scope})\n");
    }
    let pct = |num: u64, den: u64| {
        if den == 0 {
            "n/a".to_string()
        } else {
            format!("{:.1}%", num as f64 * 100.0 / den as f64)
        }
    };
    let mean = |total: u64, n: u64| {
        if n == 0 {
            "n/a".to_string()
        } else {
            format!("{:.0}", total as f64 / n as f64)
        }
    };
    format!(
        "codemode — {scope}\n\n\
         run_code calls        {}\n\
         script errors         {} ({})\n\
         inner tool calls      {} ({} per run)\n\
         output tokens / step  {} on {} run_code steps, {} on {} other tool steps\n\n\
         A script error is a thrown error, a parse error or a deadline; inner-call failures that \
         the script handled do not count. Inner tool calls include any plain call made in the \
         same step.\n",
        stats.runs,
        stats.script_errors,
        pct(stats.script_errors, stats.runs),
        stats.inner_calls,
        mean(stats.inner_calls, stats.runs),
        mean(stats.step_output_tokens, stats.steps),
        stats.steps,
        mean(stats.other_tool_step_output_tokens, stats.other_tool_steps),
        stats.other_tool_steps,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_report_shows_error_rate_and_output_tokens_per_step() {
        let stats = forge_store::CodemodeStats {
            runs: 4,
            script_errors: 1,
            inner_calls: 10,
            steps: 4,
            step_output_tokens: 1_200,
            other_tool_steps: 10,
            other_tool_step_output_tokens: 800,
        };
        let out = render(&stats, None, 30);
        assert!(out.contains("1 (25.0%)"), "{out}");
        assert!(out.contains("300 on 4 run_code steps"), "{out}");
        assert!(out.contains("80 on 10 other tool steps"), "{out}");
        assert!(render(&forge_store::CodemodeStats::default(), None, 7).contains("no run_code"));
    }
}
