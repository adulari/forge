//! `!command` typed in the composer (Claude Code's shell mode): the user's own command runs
//! through the session's `shell` tool — same workspace, sandbox, timeout and output budget as a
//! model call — and shows as an ordinary tool card. It skips the approval prompt (the user typed
//! it) but not the structural safety floor: privilege escalation and catastrophic deletes are
//! refused here exactly as they are for the model. The run then joins the conversation as a user
//! message, so the model can answer it (`[tui] respond_to_bash`) or see it next turn.

use serde_json::json;

use crate::{permission, CoreError, Message, PresenterEvent, Role, Session};

/// The longest a `!command` may run before the shell tool kills it (the tool's own ceiling).
const USER_SHELL_TIMEOUT_SECS: u64 = 600;

/// What one `!command` produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserShellRun {
    pub command: String,
    /// The shell tool's model-facing result (`shell: exit 0 in 5ms` plus output), or the refusal.
    pub output: String,
    /// It ran and exited 0.
    pub ok: bool,
    /// The safety floor refused it; nothing ran.
    pub refused: bool,
}

impl UserShellRun {
    /// The user message that carries the run into the conversation, in the `<bash-input>` /
    /// `<bash-stdout>` shape Claude Code uses, so a model knows the user ran it, not itself.
    pub fn context_message(&self) -> String {
        format!(
            "The user ran this shell command themselves:\n<bash-input>{}</bash-input>\n<bash-stdout>\n{}\n</bash-stdout>",
            self.command,
            self.output.trim_end()
        )
    }
}

/// The safety floor's verdict on a `!command`, worded for the person who typed it: the card's
/// summary and its detail. The model-facing wording (`permission::denial_note`) tells a model to
/// hand the command over; here the user already is the one to run it.
fn floor_refusal(command: &str) -> Option<(&'static str, String)> {
    if let Some(binary) = permission::privilege_escalation(command) {
        return Some((
            "needs root — run it in your own terminal",
            format!(
                "Forge never runs `{binary}`: it has no terminal to ask for the password, and every \
                 failed attempt counts toward the account lockout. Run it in your own terminal:\n\n    {}",
                command.trim()
            ),
        ));
    }
    permission::catastrophic_shell(command).map(|why| {
        (
            "refused by the safety floor",
            format!(
                "Forge's safety floor never runs this command: it {why}. If you really mean it, \
                 run it yourself outside Forge."
            ),
        )
    })
}

impl Session {
    /// Run `command` for the user (see the module docs) and draw it as a `shell` tool card.
    pub async fn run_user_shell(&mut self, command: &str) -> Result<UserShellRun, CoreError> {
        let shown = json!({ "command": command });
        self.presenter.emit(PresenterEvent::ToolStart {
            name: "shell".to_string(),
            args: shown.to_string(),
        });
        if let Some((summary, detail)) = floor_refusal(command) {
            self.presenter.emit(PresenterEvent::ToolResult {
                name: "shell".to_string(),
                ok: false,
                summary: summary.to_string(),
                detail: Some(detail.clone()),
            });
            return Ok(UserShellRun {
                command: command.to_string(),
                output: detail,
                ok: false,
                refused: true,
            });
        }
        let args = json!({
            "command": command,
            "cwd": self.workspace.root().display().to_string(),
            "timeout_secs": USER_SHELL_TIMEOUT_SECS,
        });
        let output = match self.tools.get("shell") {
            Some(tool) => match tool.run_full(&args).await {
                Ok(run) => run.model,
                Err(e) => format!("error: {e}"),
            },
            None => "error: the shell tool is not available in this session".to_string(),
        };
        let ok =
            !output.starts_with("error:") && !crate::completeness::shell_command_did_fail(&output);
        self.presenter.emit(PresenterEvent::ToolResult {
            name: "shell".to_string(),
            ok,
            summary: crate::summarize(&output),
            detail: crate::tool_detail(&output),
        });
        Ok(UserShellRun {
            command: command.to_string(),
            output,
            ok,
            refused: false,
        })
    }

    /// Add a `!command` run to the conversation without a model call (`respond_to_bash = false`):
    /// the next prompt's turn sees it like any earlier user message.
    pub fn record_user_shell(&mut self, run: &UserShellRun) -> Result<(), CoreError> {
        let mode = format!("{:?}", self.mode);
        self.store
            .ensure_session(&self.id, &self.workspace.display(), &mode)?;
        let text = run.context_message();
        let seq = self.next_seq();
        self.store
            .add_message(&self.id, seq, Role::User, &text, None)?;
        self.transcript.push(Message::user(&text));
        Ok(())
    }
}
