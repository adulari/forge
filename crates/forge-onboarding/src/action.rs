//! The things onboarding does: every change a step can make, as serializable data. A front end
//! builds an [`Action`] and hands it to [`Jobs::run`]; the engine runs it on a thread and reports
//! progress as [`crate::JobEvent`]s. Actions that must own the terminal (a browser sign-in, a CLI
//! login) are described by [`Action::exec`] so the host can suspend its screen and run them.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::host::Host;
use crate::job::{ActionResult, JobHandle, JobId, Jobs, Success};
use crate::local_actions;
use crate::state::VerifyStatus;
use crate::verify;

/// A value that must never be logged, echoed or serialized. It deserializes from the real text
/// (so a front end can send it) and serializes as `***`.
#[derive(Clone, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into().trim().to_string())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.chars().count()
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl Serialize for Secret {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str("***")
    }
}

/// Where a pasted secret is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretSlot {
    /// The dedicated token the Claude bridge runs under (from `claude setup-token`).
    ClaudeOauthToken,
    /// The Brave Search key.
    Brave,
    /// A personal Artificial Analysis key for benchmark data.
    ArtificialAnalysis,
}

impl SecretSlot {
    pub fn name(self) -> &'static str {
        match self {
            SecretSlot::ClaudeOauthToken => forge_config::CLAUDE_OAUTH_TOKEN_SLOT,
            SecretSlot::Brave => "brave",
            SecretSlot::ArtificialAnalysis => "artificialanalysis",
        }
    }
}

/// How far the background service listens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    /// This computer only.
    Local,
    /// Also the home network (phone over Wi-Fi), behind a token and self-signed HTTPS.
    Lan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum Action {
    /// Save a provider key, then check it live.
    StoreKey {
        provider: String,
        key: Secret,
        /// Replace every stored key instead of adding another.
        #[serde(default)]
        replace: bool,
    },
    StoreSecret {
        slot: SecretSlot,
        value: Secret,
    },
    /// Check a model source live (`claude-cli`, `groq`, `codex-oauth`, `ollama`, ...).
    Verify {
        source: String,
    },
    SetBehavior {
        /// `PermissionMode::key()`.
        permission: String,
        /// `normal`, `frugal` or `strict`.
        credit_mode: String,
    },
    SetKeybindPreset {
        /// `forge` or `claude-code`.
        preset: String,
    },
    SetNotifications {
        enabled: bool,
    },
    /// Turn one `[anywhere.account_sync]` category on or off for this device.
    SetAccountSync {
        category: String,
        enabled: bool,
    },
    SetTheme {
        /// `auto`, `dark` or `light`.
        theme: String,
    },
    SetLocalModel {
        tag: String,
        autostart: bool,
    },
    /// `auto`, `ask` or `off`.
    SetReporting {
        mode: String,
    },
    /// Connect Claude Code in one action: run `claude setup-token` in a pseudo-terminal, surface
    /// the sign-in URL, capture the issued token (never shown), store it and check it.
    ConnectClaude,
    /// Sign in to ChatGPT with Forge's own OAuth flow, run in a pseudo-terminal.
    CodexOauth,
    /// Sign in to xAI with Forge's own OAuth flow, run in a pseudo-terminal.
    XaiOauth,
    OllamaInstall,
    OllamaPull {
        tag: String,
    },
    /// `forge import claude|codex|cursor`: commands, skills, agents, settings.
    ImportConfig {
        tool: String,
    },
    /// `forge mcp import`: pick MCP servers from other tools (terminal: it asks which).
    ImportMcp,
    InstallService {
        exposure: Exposure,
    },
    /// `forge anywhere setup` (terminal: GitHub sign-in).
    AnywhereSetup,
    InstallDesktop,
    /// Start a local SearXNG container for keyless search.
    StartSearxng,
}

/// A program to run, `None` meaning the `forge` executable itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cmd {
    pub program: Option<String>,
    pub args: Vec<String>,
}

impl Cmd {
    fn forge(args: &[&str]) -> Cmd {
        Cmd {
            program: None,
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn named(program: &str, args: &[&str]) -> Cmd {
        Cmd {
            program: Some(program.to_string()),
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    /// The command as a person would type it.
    pub fn display(&self) -> String {
        let program = self.program.as_deref().unwrap_or("forge");
        std::iter::once(program.to_string())
            .chain(self.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// How an action runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exec {
    /// Inside the engine.
    InProcess,
    /// A child run in a pseudo-terminal: its sign-in URL, device code and prompts become job
    /// events, and a code can be typed back with [`Jobs::send_input`].
    Pty(Cmd),
    /// A child process whose output streams into the job.
    Child(Cmd),
    /// A child that needs the real terminal (browser hand-off, interactive login, a picker).
    Terminal(Cmd),
    /// Cannot run here, with the reason.
    Unavailable(String),
}

impl Action {
    /// A short label for the job list: "Checking groq".
    pub fn label(&self) -> String {
        match self {
            Action::StoreKey { provider, .. } => format!("Saving and checking the {provider} key"),
            Action::StoreSecret { slot, .. } => format!("Saving the {} secret", slot.name()),
            Action::Verify { source } => format!("Checking {source}"),
            Action::SetBehavior { .. } => "Saving safety and cost settings".into(),
            Action::SetKeybindPreset { .. } => "Saving key bindings".into(),
            Action::SetNotifications { .. } => "Saving notification settings".into(),
            Action::SetAccountSync { .. } => "Saving Anywhere sync settings".into(),
            Action::SetTheme { .. } => "Saving the theme".into(),
            Action::SetLocalModel { .. } => "Saving the local model".into(),
            Action::SetReporting { .. } => "Saving the error report setting".into(),
            Action::ConnectClaude => "Connecting Claude Code".into(),
            Action::CodexOauth => "Signing in to ChatGPT".into(),
            Action::XaiOauth => "Signing in to xAI".into(),
            Action::OllamaInstall => "Installing Ollama".into(),
            Action::OllamaPull { tag } => format!("Downloading {tag}"),
            Action::ImportConfig { tool } => format!("Importing from {tool}"),
            Action::ImportMcp => "Importing MCP servers".into(),
            Action::InstallService { .. } => "Installing the background service".into(),
            Action::AnywhereSetup => "Setting up Forge Anywhere".into(),
            Action::InstallDesktop => "Installing the desktop app".into(),
            Action::StartSearxng => "Starting local search".into(),
        }
    }

    /// Every value that must be scrubbed from this action's job output.
    pub fn secrets(&self) -> Vec<String> {
        match self {
            Action::StoreKey { key, .. } => vec![key.expose().to_string()],
            Action::StoreSecret { value, .. } => vec![value.expose().to_string()],
            _ => Vec::new(),
        }
    }

    pub fn exec(&self, host: &dyn Host) -> Exec {
        match self {
            Action::CodexOauth => Exec::Pty(Cmd::forge(&["auth", "codex-oauth"])),
            Action::XaiOauth => Exec::Pty(Cmd::forge(&["auth", "xai-oauth"])),
            Action::AnywhereSetup => Exec::Terminal(Cmd::forge(&["anywhere", "setup"])),
            Action::ImportMcp => Exec::Terminal(Cmd::forge(&["mcp", "import"])),
            Action::OllamaInstall => match host.ollama_install_argv() {
                Some(argv) if !argv.is_empty() => Exec::Terminal(Cmd {
                    program: Some(argv[0].clone()),
                    args: argv[1..].to_vec(),
                }),
                _ => Exec::Unavailable(
                    "There is no unattended Ollama install for this system. Download it from https://ollama.com/download."
                        .into(),
                ),
            },
            Action::OllamaPull { tag } => Exec::Child(Cmd::named("ollama", &["pull", tag])),
            Action::ImportConfig { tool } => match tool.as_str() {
                "claude" | "codex" | "cursor" => Exec::Child(Cmd::forge(&["import", tool])),
                other => Exec::Unavailable(format!("Forge cannot import from {other}.")),
            },
            Action::InstallService { exposure } => Exec::Child(Cmd::forge(&[
                "service",
                "install",
                match exposure {
                    Exposure::Local => "--local",
                    Exposure::Lan => "--lan",
                },
            ])),
            Action::InstallDesktop => Exec::Child(desktop_installer()),
            _ => Exec::InProcess,
        }
    }
}

const DESKTOP_INSTALL_SH: &str =
    "https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.sh";
const DESKTOP_INSTALL_PS1: &str =
    "https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.ps1";

fn desktop_installer() -> Cmd {
    if cfg!(windows) {
        Cmd::named(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                &format!("irm {DESKTOP_INSTALL_PS1} | iex"),
            ],
        )
    } else {
        Cmd::named(
            "sh",
            &["-c", &format!("curl -fsSL {DESKTOP_INSTALL_SH} | sh")],
        )
    }
}

impl Jobs {
    /// Start `action` as a job. An action that needs the terminal does not run here: the job
    /// ends at once with the command to run, so a daemon front end never hangs on it.
    pub fn run(&self, action: Action, host: Arc<dyn Host>) -> JobId {
        let label = action.label();
        match action.exec(host.as_ref()) {
            Exec::Terminal(cmd) => self.finished(
                label,
                Err(format!(
                    "This step needs a terminal. Run `{}` there, then come back.",
                    cmd.display()
                )),
            ),
            Exec::Unavailable(reason) => self.finished(label, Err(reason)),
            Exec::Pty(cmd) => {
                let source = match action {
                    Action::CodexOauth => "codex-oauth",
                    _ => "xai-oauth",
                };
                self.start(label, Vec::new(), move |job| {
                    crate::flows::oauth_sign_in(job, &host, &cmd, source)
                })
            }
            Exec::Child(cmd) => {
                let program = cmd
                    .program
                    .clone()
                    .or_else(|| host.forge_exe().map(|p| p.display().to_string()));
                self.start(label, Vec::new(), move |job| {
                    let program = program.ok_or("Forge cannot find its own executable.")?;
                    crate::proc::stream_child(job, &program, &cmd.args)?;
                    Ok(Success::with("Done.", ActionResult::Changed))
                })
            }
            Exec::InProcess => {
                let secrets = action.secrets();
                self.start(label, secrets, move |job| {
                    run_in_process(&action, &host, job)
                })
            }
        }
    }
}

fn run_in_process(
    action: &Action,
    host: &Arc<dyn Host>,
    job: &JobHandle,
) -> Result<Success, String> {
    match action {
        Action::Verify { source } => verify::verify_source(source, job),
        Action::StoreKey {
            provider,
            key,
            replace,
        } => {
            local_actions::store_key(provider, key, *replace)?;
            host.credentials_changed(provider);
            job.output("Saved. Checking it now.");
            let mut outcome = verify::verify_source(provider, job)?;
            if matches!(
                outcome.result,
                Some(ActionResult::Verified {
                    status: VerifyStatus::Rejected,
                    ..
                })
            ) {
                outcome.summary = format!(
                    "{} The key is saved; replace it to try another.",
                    outcome.summary
                );
            }
            Ok(outcome)
        }
        Action::StoreSecret { slot, value } => {
            local_actions::store_secret(*slot, value)?;
            host.credentials_changed(match slot {
                SecretSlot::ClaudeOauthToken => "claude-cli",
                SecretSlot::Brave => "brave",
                SecretSlot::ArtificialAnalysis => "artificialanalysis",
            });
            Ok(Success::with("Saved.", ActionResult::Changed))
        }
        Action::SetBehavior {
            permission,
            credit_mode,
        } => local_actions::set_behavior(permission, credit_mode),
        Action::SetKeybindPreset { preset } => local_actions::set_keybind_preset(preset),
        Action::SetNotifications { enabled } => local_actions::set_notifications(*enabled),
        Action::SetAccountSync { category, enabled } => {
            local_actions::set_account_sync(category, *enabled)
        }
        Action::SetTheme { theme } => local_actions::set_theme(theme),
        Action::SetLocalModel { tag, autostart } => local_actions::set_local_model(tag, *autostart),
        Action::SetReporting { mode } => crate::reporting::set_mode(mode)
            .map(|()| Success::with(format!("Error reports: {mode}."), ActionResult::Changed)),
        Action::StartSearxng => local_actions::start_searxng(job),
        Action::ConnectClaude => crate::flows::connect_claude(job, host),
        // Every other action is a child process and never reaches here.
        other => Err(format!("{} is not an in-process step.", other.label())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::NoHost;
    use crate::job::{JobEvent, JobState};
    use std::time::{Duration, Instant};

    fn wait(jobs: &Jobs, id: JobId) {
        let start = Instant::now();
        while jobs.state(id) == Some(JobState::Running) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "job never finished"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn secrets_debug_and_serialize_as_stars_but_deserialize_from_text() {
        let action = Action::StoreKey {
            provider: "groq".into(),
            key: Secret::new("  gsk-live-0123456789  "),
            replace: false,
        };
        assert!(!format!("{action:?}").contains("gsk-live"));
        let json = serde_json::to_string(&action).unwrap();
        assert!(!json.contains("gsk-live"), "{json}");
        assert!(json.contains("***"));
        let parsed: Action = serde_json::from_str(
            r#"{"action":"store_key","provider":"groq","key":"gsk-live-0123456789"}"#,
        )
        .unwrap();
        let Action::StoreKey { key, replace, .. } = parsed else {
            panic!("wrong variant");
        };
        assert_eq!(key.expose(), "gsk-live-0123456789");
        assert!(!replace);
    }

    #[test]
    fn actions_serialize_with_an_action_tag_for_the_daemon() {
        let json = serde_json::to_value(Action::InstallService {
            exposure: Exposure::Local,
        })
        .unwrap();
        assert_eq!(json["action"], "install_service");
        assert_eq!(json["exposure"], "local");
        let back: Action = serde_json::from_value(json).unwrap();
        assert_eq!(
            back,
            Action::InstallService {
                exposure: Exposure::Local
            }
        );
    }

    #[test]
    fn interactive_actions_hand_the_terminal_over_and_the_rest_run_in_the_engine() {
        for action in [Action::AnywhereSetup, Action::ImportMcp] {
            assert!(
                matches!(action.exec(&NoHost), Exec::Terminal(_)),
                "{action:?}"
            );
        }
        for action in [Action::CodexOauth, Action::XaiOauth] {
            assert!(matches!(action.exec(&NoHost), Exec::Pty(_)), "{action:?}");
        }
        assert!(matches!(
            Action::ConnectClaude.exec(&NoHost),
            Exec::InProcess
        ));
        assert!(matches!(
            Action::Verify {
                source: "groq".into()
            }
            .exec(&NoHost),
            Exec::InProcess
        ));
        assert!(matches!(
            Action::InstallService {
                exposure: Exposure::Local
            }
            .exec(&NoHost),
            Exec::Child(_)
        ));
    }

    #[test]
    fn the_service_install_command_carries_the_chosen_exposure() {
        let Exec::Child(local) = (Action::InstallService {
            exposure: Exposure::Local,
        })
        .exec(&NoHost) else {
            panic!()
        };
        assert_eq!(local.display(), "forge service install --local");
        let Exec::Child(lan) = (Action::InstallService {
            exposure: Exposure::Lan,
        })
        .exec(&NoHost) else {
            panic!()
        };
        assert_eq!(lan.display(), "forge service install --lan");
    }

    #[test]
    fn an_unknown_import_tool_is_refused_not_run() {
        let exec = Action::ImportConfig {
            tool: "sh -c evil".into(),
        }
        .exec(&NoHost);
        assert!(matches!(exec, Exec::Unavailable(_)));
    }

    #[test]
    fn ollama_install_is_unavailable_unless_the_host_knows_a_command() {
        assert!(matches!(
            Action::OllamaInstall.exec(&NoHost),
            Exec::Unavailable(_)
        ));
        struct Brew;
        impl Host for Brew {
            fn ollama_install_argv(&self) -> Option<Vec<String>> {
                Some(vec!["brew".into(), "install".into(), "ollama".into()])
            }
        }
        let Exec::Terminal(cmd) = Action::OllamaInstall.exec(&Brew) else {
            panic!()
        };
        assert_eq!(cmd.display(), "brew install ollama");
    }

    #[test]
    fn a_terminal_action_run_as_a_job_fails_with_the_command_to_run() {
        let jobs = Jobs::new();
        let id = jobs.run(Action::AnywhereSetup, Arc::new(NoHost));
        wait(&jobs, id);
        let poll = jobs.poll(id, 0).unwrap();
        assert_eq!(poll.state, JobState::Failed);
        let Some(JobEvent::Failed { error, .. }) = poll.events.last() else {
            panic!()
        };
        assert!(error.contains("forge anywhere setup"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_action_streams_its_output_into_the_job() {
        struct Echo;
        impl Host for Echo {
            fn forge_exe(&self) -> Option<std::path::PathBuf> {
                static EXE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
                Some(EXE.get_or_init(echo_script).clone())
            }
        }
        // A test thread that forks while the script is still open for writing holds the
        // write fd until it execs, and running the script then fails with ETXTBSY.
        fn echo_script() -> std::path::PathBuf {
            use std::os::unix::fs::PermissionsExt;
            let dir = std::env::temp_dir().join(format!("forge-onb-echo-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let exe = dir.join("forge");
            std::fs::write(&exe, "#!/bin/sh\necho \"args: $*\"\necho oops 1>&2\n").unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            for _ in 0..100 {
                match std::process::Command::new(&exe).output() {
                    Err(e) if e.raw_os_error() == Some(26) => {
                        std::thread::sleep(std::time::Duration::from_millis(20))
                    }
                    _ => break,
                }
            }
            exe
        }
        let jobs = Jobs::new();
        let id = jobs.run(
            Action::InstallService {
                exposure: Exposure::Local,
            },
            Arc::new(Echo),
        );
        wait(&jobs, id);
        let poll = jobs.poll(id, 0).unwrap();
        assert_eq!(poll.state, JobState::Succeeded, "{:?}", poll.events);
        let lines: Vec<&str> = poll
            .events
            .iter()
            .filter_map(|e| match e {
                JobEvent::Output { line, .. } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            lines.contains(&"args: service install --local"),
            "{lines:?}"
        );
        assert!(lines.contains(&"oops"), "stderr is streamed too: {lines:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_child_action_ends_failed_with_its_exit_status() {
        struct Fail;
        impl Host for Fail {
            fn forge_exe(&self) -> Option<std::path::PathBuf> {
                Some(std::path::PathBuf::from("false"))
            }
        }
        let jobs = Jobs::new();
        let id = jobs.run(
            Action::InstallService {
                exposure: Exposure::Local,
            },
            Arc::new(Fail),
        );
        wait(&jobs, id);
        assert_eq!(jobs.state(id), Some(JobState::Failed));
    }
}
