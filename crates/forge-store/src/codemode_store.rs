//! Codemode metrics read back from the audit trail (docs/features/codemode.md). A `run_code`
//! call writes its own `tool_call` row (status `ok` when the script returned, `error` when it
//! threw, failed to parse or ran out of time), and its inner calls write rows under the same
//! assistant message, whose `usage` row holds the output tokens the program cost.

use super::*;

/// Codemode adoption and cost over a window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodemodeStats {
    pub runs: u64,
    /// Runs whose script threw, failed to parse, or hit its deadline.
    pub script_errors: u64,
    /// Tool calls recorded on steps that ran a script, the `run_code` rows excluded. Inner calls,
    /// plus any plain call the model made in the same step.
    pub inner_calls: u64,
    /// Model steps that emitted at least one `run_code`, and their output tokens.
    pub steps: u64,
    pub step_output_tokens: u64,
    /// Every other model step that called a tool, for comparison.
    pub other_tool_steps: u64,
    pub other_tool_step_output_tokens: u64,
}

/// One audit row, for tests and diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRow {
    pub tool_name: String,
    pub permission: String,
    pub status: String,
}

const RUN_CODE_STEPS: &str = "SELECT message_id FROM tool_call WHERE tool_name = 'run_code'";

impl Store {
    /// Codemode metrics since `since` (unix seconds), for one session or all of them.
    pub fn codemode_stats(&self, session_id: Option<&str>, since: i64) -> Result<CodemodeStats> {
        let conn = self.lock()?;
        let (runs, script_errors): (i64, i64) = conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(t.status = 'error'), 0)
             FROM tool_call t JOIN message m ON m.id = t.message_id
             WHERE t.tool_name = 'run_code' AND t.created_at >= ?1
               AND (?2 IS NULL OR m.session_id = ?2)",
            rusqlite::params![since, session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let inner_calls: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM tool_call t JOIN message m ON m.id = t.message_id
                 WHERE t.tool_name != 'run_code' AND t.message_id IN ({RUN_CODE_STEPS})
                   AND t.created_at >= ?1 AND (?2 IS NULL OR m.session_id = ?2)"
            ),
            rusqlite::params![since, session_id],
            |r| r.get(0),
        )?;
        let steps = |membership: &str| -> Result<(i64, i64)> {
            Ok(conn.query_row(
                &format!(
                    "SELECT COUNT(*), COALESCE(SUM(u.output_tokens), 0)
                     FROM usage u JOIN message m ON m.id = u.message_id
                     WHERE m.role = 'assistant' AND {membership}
                       AND u.created_at >= ?1 AND (?2 IS NULL OR m.session_id = ?2)"
                ),
                rusqlite::params![since, session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        };
        let (code_steps, code_tokens) = steps(&format!("u.message_id IN ({RUN_CODE_STEPS})"))?;
        let (other_steps, other_tokens) = steps(&format!(
            "u.message_id IN (SELECT message_id FROM tool_call)
             AND u.message_id NOT IN ({RUN_CODE_STEPS})"
        ))?;
        let n = |v: i64| v.max(0) as u64;
        Ok(CodemodeStats {
            runs: n(runs),
            script_errors: n(script_errors),
            inner_calls: n(inner_calls),
            steps: n(code_steps),
            step_output_tokens: n(code_tokens),
            other_tool_steps: n(other_steps),
            other_tool_step_output_tokens: n(other_tokens),
        })
    }

    /// Every audit row recorded under one assistant message, in insertion order.
    pub fn tool_call_rows(&self, message_id: &str) -> Result<Vec<ToolCallRow>> {
        let conn = self.lock()?;
        let mut stmt = conn.prepare(
            "SELECT tool_name, permission, status FROM tool_call
             WHERE message_id = ?1 ORDER BY rowid",
        )?;
        let rows = stmt
            .query_map([message_id], |r| {
                Ok(ToolCallRow {
                    tool_name: r.get(0)?,
                    permission: r.get(1)?,
                    status: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_code_runs_errors_and_step_tokens_are_counted() {
        let store = Store::open_in_memory().unwrap();
        let sid = store.create_session("/tmp", "default").unwrap();
        let code_step = store
            .add_message(&sid, 0, Role::Assistant, "", None)
            .unwrap();
        let plain_step = store
            .add_message(&sid, 1, Role::Assistant, "", None)
            .unwrap();
        for (name, status) in [
            ("read_file", "ok"),
            ("edit_file", "ok"),
            ("run_code", "ok"),
            ("run_code", "error"),
        ] {
            store
                .record_tool_call(&code_step, name, "{}", "r", "allowed", status)
                .unwrap();
        }
        store
            .record_tool_call(&plain_step, "shell", "{}", "r", "allowed", "ok")
            .unwrap();
        let usage = |out| Usage {
            input_tokens: 100,
            output_tokens: out,
            ..Default::default()
        };
        store
            .record_usage(&sid, &code_step, &usage(300), Some("direct::m"))
            .unwrap();
        store
            .record_usage(&sid, &plain_step, &usage(40), Some("direct::m"))
            .unwrap();

        let stats = store.codemode_stats(Some(&sid), 0).unwrap();
        assert_eq!(stats.runs, 2);
        assert_eq!(stats.script_errors, 1);
        assert_eq!(stats.inner_calls, 2);
        assert_eq!((stats.steps, stats.step_output_tokens), (1, 300));
        assert_eq!(
            (stats.other_tool_steps, stats.other_tool_step_output_tokens),
            (1, 40)
        );
        assert_eq!(store.codemode_stats(Some("other"), 0).unwrap().runs, 0);
        let rows = store.tool_call_rows(&code_step).unwrap();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[3].status, "error");
    }
}
