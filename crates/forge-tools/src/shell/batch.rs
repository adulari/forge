//! `shell` with `commands: [...]`: several commands in one tool call, run in order.
//!
//! A model doing multi-step shell work used to spend one round trip per command (hundreds of calls
//! in a long session, each re-sending the whole context). A batch runs the entries sequentially,
//! each with the plain path's full semantics (own process group, timeout, sandbox, RTK rewrite),
//! and returns one result with a section per entry. The first line keeps the single-command shape
//! (`shell: exit N in Xms`) with the first failing entry's exit code, so every caller that reads
//! the exit status (verification, failure tracking, shell diagnosis) works unchanged.

use super::*;

/// Most entries one batch may carry.
pub(super) const MAX_BATCH_COMMANDS: usize = 20;
/// Smallest share of the model budget any one entry is cut to.
const MIN_ENTRY_BUDGET: usize = 2 * 1024;
/// Exit code reported for an entry that timed out (the `timeout(1)` convention).
const TIMED_OUT_CODE: i32 = 124;

/// The entries of a `commands` argument, validated.
pub(super) fn batch_commands(list: &Value) -> Result<Vec<String>, String> {
    let Some(items) = list.as_array() else {
        return Err("shell: `commands` must be an array of command strings".to_string());
    };
    if items.is_empty() {
        return Err("shell: `commands` is empty".to_string());
    }
    if items.len() > MAX_BATCH_COMMANDS {
        return Err(format!(
            "shell: `commands` holds {} entries; the limit is {MAX_BATCH_COMMANDS}. Split the \
             batch, or put the steps in a script file and run that.",
            items.len()
        ));
    }
    items
        .iter()
        .map(|item| match item.as_str().map(str::trim) {
            Some(cmd) if !cmd.is_empty() => Ok(cmd.to_string()),
            _ => Err("shell: every `commands` entry must be a non-empty string".to_string()),
        })
        .collect()
}

/// The exit code a single-command result reports, or `None` when it did not exit normally.
fn entry_exit_code(result: &str) -> Option<i32> {
    result
        .lines()
        .next()?
        .strip_prefix("shell: exit ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// One finished entry, before the batch budget is applied.
struct EntryRun {
    command: String,
    result: String,
    code: i32,
}

impl ShellTool {
    pub(super) async fn run_batch(
        &self,
        commands: Vec<String>,
        cwd: &str,
        timeout_secs: u64,
        stop_on_error: bool,
    ) -> crate::ToolRun {
        let started = Instant::now();
        let total = commands.len();
        let mut runs = Vec::with_capacity(total);
        for command in commands {
            let (result, _) = match self.rtk.as_ref().and_then(|r| r.rewrite(&command)) {
                Some(rewritten) => {
                    let (result, full) =
                        run_command_full(&rewritten, cwd, timeout_secs, &self.policy).await;
                    (rtk::tag_result(result), full)
                }
                None => run_command_full(&command, cwd, timeout_secs, &self.policy).await,
            };
            let code = entry_exit_code(&result).unwrap_or_else(|| {
                if result.starts_with("shell: timed out") {
                    TIMED_OUT_CODE
                } else {
                    1
                }
            });
            let failed = code != 0;
            runs.push(EntryRun {
                command,
                result,
                code,
            });
            if failed && stop_on_error {
                break;
            }
        }
        crate::ToolRun::from(render_batch(&runs, total, started.elapsed()))
    }
}

/// The batch result: a summary header, then one section per entry that ran. Each entry is cut to
/// its share of [`MODEL_BUDGET`]; a failed entry gets twice the share of a passing one, since its
/// output is the part the model has to read.
fn render_batch(runs: &[EntryRun], total: usize, elapsed: Duration) -> String {
    let ok = runs.iter().filter(|r| r.code == 0).count();
    let first_failure = runs.iter().position(|r| r.code != 0);
    let code = first_failure.map_or(0, |i| runs[i].code);
    let mut header = format!(
        "shell: exit {code} in {}ms ({ok}/{total} ok",
        elapsed.as_millis()
    );
    if runs.len() < total {
        header.push_str(&format!(", stopped at #{}", runs.len()));
    }
    header.push(')');
    let weight = |r: &EntryRun| if r.code == 0 { 1 } else { 2 };
    let weights: usize = runs.iter().map(weight).sum::<usize>().max(1);
    let mut out = header;
    for (i, run) in runs.iter().enumerate() {
        let budget = (MODEL_BUDGET * weight(run) / weights).max(MIN_ENTRY_BUDGET);
        let (body, _) = truncate_for_model(&run.result, budget);
        out.push_str(&format!(
            "\n\n── [{}] {}\n{}",
            i + 1,
            run.command,
            body.trim_end()
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ShellTool {
        ShellTool::default()
    }

    #[test]
    fn rejects_empty_oversized_and_non_string_batches() {
        assert!(batch_commands(&json!([])).is_err());
        assert!(batch_commands(&json!("ls")).is_err());
        assert!(batch_commands(&json!(["ls", 3])).is_err());
        assert!(batch_commands(&json!(["ls", "  "])).is_err());
        let many: Vec<String> = (0..=MAX_BATCH_COMMANDS)
            .map(|i| format!("echo {i}"))
            .collect();
        assert!(batch_commands(&json!(many)).is_err());
        assert_eq!(
            batch_commands(&json!(["echo a", " echo b "])).unwrap(),
            vec!["echo a", "echo b"]
        );
    }

    #[tokio::test]
    async fn runs_entries_in_order_and_reports_each() {
        let out = tool()
            .run(&json!({ "commands": ["echo first", "echo second"] }))
            .await
            .unwrap();
        let first_line = out.lines().next().unwrap();
        assert!(first_line.starts_with("shell: exit 0 in "), "{out}");
        assert!(first_line.ends_with("(2/2 ok)"), "{out}");
        let a = out.find("── [1] echo first").expect("first section");
        let b = out.find("── [2] echo second").expect("second section");
        assert!(a < b);
        assert!(out.contains("first") && out.contains("second"));
    }

    #[tokio::test]
    async fn stops_at_the_first_failure_by_default_and_reports_its_code() {
        let out = tool()
            .run(&json!({ "commands": ["echo ok", "exit 3", "echo never"] }))
            .await
            .unwrap();
        let first_line = out.lines().next().unwrap();
        assert!(first_line.starts_with("shell: exit 3 in "), "{out}");
        assert!(first_line.ends_with("(1/3 ok, stopped at #2)"), "{out}");
        assert!(!out.contains("── [3]"), "{out}");
    }

    #[tokio::test]
    async fn keeps_going_when_asked_and_still_fails_the_call() {
        let out = tool()
            .run(&json!({ "commands": ["exit 2", "echo after"], "stop_on_error": false }))
            .await
            .unwrap();
        let first_line = out.lines().next().unwrap();
        assert!(first_line.starts_with("shell: exit 2 in "), "{out}");
        assert!(first_line.ends_with("(1/2 ok)"), "{out}");
        assert!(out.contains("── [2] echo after"), "{out}");
    }

    #[tokio::test]
    async fn single_command_modes_are_refused_with_a_batch() {
        for extra in [
            json!({ "background": true }),
            json!({ "pty": true }),
            json!({ "poll_until_exit_zero": true }),
        ] {
            let mut args = json!({ "commands": ["echo a"] });
            args.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let out = tool().run(&args).await.unwrap();
            assert!(out.contains("not supported with `commands`"), "{out}");
        }
    }

    #[test]
    fn a_failed_entry_gets_the_larger_share_of_the_budget() {
        let big = "x".repeat(MODEL_BUDGET);
        let runs = vec![
            EntryRun {
                command: "pass".into(),
                result: format!("shell: exit 0 in 1ms\n{big}"),
                code: 0,
            },
            EntryRun {
                command: "fail".into(),
                result: format!("shell: exit 1 in 1ms\n{big}"),
                code: 1,
            },
        ];
        let out = render_batch(&runs, 2, Duration::from_millis(2));
        let pass_len = out.find("── [2]").unwrap() - out.find("── [1]").unwrap();
        let fail_len = out.len() - out.find("── [2]").unwrap();
        assert!(fail_len > pass_len, "pass {pass_len} fail {fail_len}");
        assert!(out.len() <= MODEL_BUDGET + 512);
    }
}
