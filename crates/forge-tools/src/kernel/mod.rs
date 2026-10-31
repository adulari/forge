//! The `python` tool: a persistent, Jupyter-style Python kernel per session, gated by
//! `[harness] eval_kernel`. Variables, imports and loaded data survive between calls until a
//! reset, a crash, a timeout kill or the end of the session. It is an ordinary `Tool` with
//! `SideEffect::Shell`, so the broker, hooks, audit row and full-output spill already apply.

/// The tool's registry name. Compiled everywhere so forge-core builds on Windows.
pub const PYTHON_TOOL: &str = "python";

#[cfg(unix)]
mod discover;
#[cfg(unix)]
mod output;
#[cfg(unix)]
mod process;
#[cfg(all(test, unix))]
mod tests;

#[cfg(unix)]
pub use discover::discover_python;
#[cfg(unix)]
pub use tool::PythonTool;

#[cfg(unix)]
mod tool {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use forge_types::SideEffect;
    use serde_json::{json, Value};

    use super::discover::discover_python;
    use super::output::{self, Rendered};
    use super::process::{Kernel, Outcome};
    use super::PYTHON_TOOL;
    use crate::{Tool, ToolError, ToolRun, SESSION_WORKSPACE};

    pub(super) const DEFAULT_TIMEOUT_SECS: u64 = 60;
    pub(super) const MAX_TIMEOUT_SECS: u64 = 600;
    const STARTUP_BUDGET: Duration = Duration::from_secs(20);
    const NEW_KERNEL: &str = "new kernel: no variables from earlier calls exist";
    const RESET_NOTICE: &str = "fresh kernel (reset): earlier variables are gone";

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct Cell {
        pub code: String,
        pub timeout_secs: u64,
        pub reset: bool,
    }

    pub(super) fn parse_args(args: &Value) -> Result<Cell, ToolError> {
        let raw = args
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::Failed("python: `code` (string) is required".into()))?;
        let code = strip_fence(raw);
        if code.trim().is_empty() {
            return Err(ToolError::Failed("python: `code` is empty".into()));
        }
        let timeout_secs = args
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        Ok(Cell {
            code,
            timeout_secs,
            reset: args.get("reset").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// One leading ```python (or bare ```) fence and its closing fence.
    pub(super) fn strip_fence(code: &str) -> String {
        let trimmed = code.trim();
        let Some(rest) = trimmed.strip_prefix("```") else {
            return code.to_string();
        };
        let body = match rest.split_once('\n') {
            Some((tag, body)) if tag.trim().is_empty() || tag.trim().starts_with("py") => body,
            _ => return code.to_string(),
        };
        body.trim_end()
            .strip_suffix("```")
            .unwrap_or(body)
            .trim_end()
            .to_string()
    }

    #[derive(Default)]
    struct KernelSlot {
        kernel: Option<Kernel>,
        notice: Option<String>,
    }

    pub struct PythonTool {
        slot: Arc<tokio::sync::Mutex<KernelSlot>>,
    }

    impl Default for PythonTool {
        fn default() -> Self {
            Self::new()
        }
    }

    impl PythonTool {
        pub fn new() -> Self {
            Self {
                slot: Arc::new(tokio::sync::Mutex::new(KernelSlot::default())),
            }
        }
    }

    async fn start(workspace: &std::path::Path) -> Result<Kernel, String> {
        let candidates = discover_python(workspace);
        let mut tried = Vec::new();
        let mut last = String::new();
        let started = tokio::time::Instant::now();
        for python in candidates {
            let left = STARTUP_BUDGET.saturating_sub(started.elapsed());
            if left.is_zero() {
                break;
            }
            tried.push(python.display().to_string());
            match Kernel::spawn(&python, workspace).await {
                Ok(mut kernel) => match kernel.hello(left).await {
                    Ok(_) => return Ok(kernel),
                    Err(why) => last = why,
                },
                Err(why) => last = why,
            }
        }
        if tried.is_empty() {
            tried.push("no interpreter".into());
        }
        Err(output::failed_header(&tried, &last))
    }

    #[async_trait]
    impl Tool for PythonTool {
        fn name(&self) -> &str {
            PYTHON_TOOL
        }

        fn description(&self) -> &str {
            "Run Python in a persistent kernel, like a Jupyter notebook. Variables, imports and loaded data survive between calls in this session: load once (`df = pd.read_csv('orders.csv')`), query in later calls (`df.groupby('region').sum()`). print() output and a final bare expression are shown. cwd is the workspace. Use it to explore data and files and to try code; kernel state is not the deliverable: write final code and outputs to disk and run them with `shell`. Run tests, builds and servers with `shell`. Edited modules are not re-imported: use importlib.reload or reset."
        }

        fn side_effect(&self) -> SideEffect {
            SideEffect::Shell
        }

        fn schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "code": { "type": "string", "description": "Python cell. A final bare expression is displayed. `!cmd` runs a shell command; `%pip install x` installs into this interpreter." },
                    "timeout_secs": { "type": "integer", "description": "Cell deadline in seconds (default 60, max 600). On timeout the cell is interrupted and earlier state is kept." },
                    "reset": { "type": "boolean", "description": "Start a fresh kernel first (all variables are lost)." }
                },
                "required": ["code"]
            })
        }

        async fn run(&self, args: &Value) -> Result<String, ToolError> {
            self.run_full(args).await.map(|run| run.model)
        }

        async fn run_full(&self, args: &Value) -> Result<ToolRun, ToolError> {
            let cell = parse_args(args)?;
            let workspace: PathBuf = SESSION_WORKSPACE
                .try_with(|w| w.clone())
                .ok()
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            let mut slot = self.slot.lock().await;

            let mut drop_reason: Option<String> = None;
            if cell.reset {
                drop_reason = Some(RESET_NOTICE.to_string());
                slot.kernel = None;
            } else if let Some(kernel) = slot.kernel.as_mut() {
                if kernel.cwd != workspace {
                    drop_reason = Some(format!("kernel restarted in {}", workspace.display()));
                } else if let Some(status) = kernel.try_exited() {
                    drop_reason = Some(exited_notice(&status));
                } else if !kernel.settle_busy().await {
                    drop_reason = Some(
                        "kernel restarted: a cell from an aborted turn was still running; all variables are gone, re-run your setup"
                            .to_string(),
                    );
                }
                if drop_reason.is_some() {
                    slot.kernel = None;
                }
            }
            if drop_reason.is_some() {
                slot.notice = drop_reason;
            }

            let mut notice = None;
            if slot.kernel.is_none() {
                match start(&workspace).await {
                    Ok(kernel) => {
                        slot.kernel = Some(kernel);
                        notice = Some(slot.notice.take().unwrap_or_else(|| NEW_KERNEL.into()));
                    }
                    Err(header) => return Ok(ToolRun::from(header)),
                }
            }

            let kernel = slot.kernel.as_mut().expect("kernel started above");
            let outcome = kernel
                .exec(&cell.code, Duration::from_secs(cell.timeout_secs))
                .await;
            let mut rendered = Rendered {
                header: String::new(),
                notice,
                output: String::new(),
                value: None,
                traceback: None,
                capture_truncated: false,
            };
            match outcome {
                Outcome::Done {
                    reply,
                    output,
                    truncated,
                    timed_out_secs,
                } => {
                    rendered.output = output;
                    rendered.capture_truncated = truncated;
                    let id = reply["id"].as_u64().unwrap_or(0);
                    rendered.header = match (output::reply_str(&reply, "status"), timed_out_secs) {
                        (Some("error"), _) => {
                            rendered.traceback =
                                output::reply_str(&reply, "traceback").map(str::to_string);
                            output::error_header(
                                id,
                                output::reply_str(&reply, "ename").unwrap_or("Error"),
                            )
                        }
                        (Some("interrupted"), secs) => {
                            output::interrupted_header(secs.unwrap_or(cell.timeout_secs))
                        }
                        _ => {
                            rendered.value =
                                output::reply_str(&reply, "result").map(str::to_string);
                            output::ok_header(
                                id,
                                reply["elapsed_ms"].as_u64().unwrap_or(0),
                                reply["bang_exit"].as_i64(),
                            )
                        }
                    };
                }
                Outcome::Killed { output, truncated } => {
                    rendered.header = output::KILLED_HEADER.to_string();
                    rendered.output = output;
                    rendered.capture_truncated = truncated;
                    slot.kernel = None;
                    slot.notice = Some(exited_notice("killed after ignoring interrupt"));
                }
                Outcome::Died {
                    status,
                    output,
                    truncated,
                } => {
                    rendered.header = output::died_header(&status);
                    rendered.output = output;
                    rendered.capture_truncated = truncated;
                    slot.kernel = None;
                    slot.notice = Some(exited_notice(&status));
                }
            }
            Ok(output::assemble(rendered))
        }
    }

    fn exited_notice(status: &str) -> String {
        format!("previous kernel exited ({status}); all variables are gone, re-run your setup")
    }
}
