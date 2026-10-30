//! The one-action sign-in flows: each runs its CLI in a pseudo-terminal, shows the sign-in URL,
//! stores what comes back and checks it. Nothing here prints a credential.

use std::sync::Arc;
use std::time::Duration;

use crate::action::{Cmd, SecretSlot};
use crate::host::Host;
use crate::job::{ActionResult, JobHandle, Success};
use crate::pty::{run_pty, PtySpec};
use crate::verify::verify_source;

const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// `claude setup-token` in a PTY, then store and check the token it issues. When the token cannot
/// be read from the output the job still succeeds, asking the front end for a paste instead.
pub(crate) fn connect_claude(job: &JobHandle, host: &Arc<dyn Host>) -> Result<Success, String> {
    let program = crate::detect::find_binary("claude")
        .ok_or("Claude Code is not installed. Install it from claude.com/code and try again.")?;
    job.progress("Opening the Claude sign-in", None);
    let args = vec!["setup-token".to_string()];
    let run = run_pty(
        job,
        PtySpec {
            program: &program,
            args: &args,
            timeout: SIGN_IN_TIMEOUT,
            env_remove: &["CLAUDE_CODE_OAUTH_TOKEN"],
        },
    )?;
    let Some(token) = run.token else {
        job.output("Claude finished, but Forge could not read the token from its output.");
        return Ok(Success::with(
            "Paste the token Claude printed.",
            ActionResult::PasteNeeded {
                slot: SecretSlot::ClaudeOauthToken,
            },
        ));
    };
    forge_config::store_api_key(forge_config::CLAUDE_OAUTH_TOKEN_SLOT, &token)
        .map_err(|e| format!("Could not save the token: {e}"))?;
    host.credentials_changed("claude-cli");
    job.output("Token saved. Checking that Claude Code answers.");
    verify_source("claude-cli", job)
}

/// Forge's own OAuth sign-in (`forge auth codex-oauth` / `xai-oauth`) in a PTY, then a live
/// check of the account.
pub(crate) fn oauth_sign_in(
    job: &JobHandle,
    host: &Arc<dyn Host>,
    cmd: &Cmd,
    source: &str,
) -> Result<Success, String> {
    let program = cmd
        .program
        .clone()
        .or_else(|| host.forge_exe().map(|p| p.display().to_string()))
        .ok_or("Forge cannot find its own executable.")?;
    job.progress("Starting the sign-in", None);
    let run = run_pty(
        job,
        PtySpec {
            program: &program,
            args: &cmd.args,
            timeout: SIGN_IN_TIMEOUT,
            env_remove: &[],
        },
    )?;
    if !run.success {
        return Err("The sign-in did not finish. You can try again.".into());
    }
    host.credentials_changed(source);
    verify_source(source, job)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::host::NoHost;
    use crate::job::{JobEvent, JobState, Jobs, PromptKind};
    use crate::test_support::{env_lock, ScopedEnv};
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    const TOKEN: &str = "sk-ant-oat01-FakeTokenForTestsOnly0123456789abcdefghijkl";

    struct Fixture {
        _guard: std::sync::MutexGuard<'static, ()>,
        _env: ScopedEnv,
        dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let guard = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let config = dir.path().join("config");
        let data = dir.path().join("data");
        let path = format!("{}:/usr/bin:/bin", bin.display());
        let env = ScopedEnv::set(&[
            ("HOME", Some(dir.path().as_os_str())),
            ("XDG_CONFIG_HOME", Some(config.as_os_str())),
            ("XDG_DATA_HOME", Some(data.as_os_str())),
            ("PATH", Some(OsStr::new(&path))),
            ("FORGE_DB", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            ("ANTHROPIC_API_KEY", None),
        ]);
        Fixture {
            _guard: guard,
            _env: env,
            dir,
        }
    }

    fn fake_claude(f: &Fixture, body: &str) {
        let path = f.dir.path().join("bin/claude");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn run_to_end(jobs: &Jobs, id: u64) -> crate::job::JobPoll {
        let start = Instant::now();
        while jobs.state(id) == Some(JobState::Running) {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "flow never finished"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        jobs.poll(id, 0).unwrap()
    }

    #[test]
    fn claude_is_connected_by_one_action_the_token_is_captured_stored_and_never_shown() {
        let f = fixture();
        fake_claude(
            &f,
            &format!(
                "echo 'Opening browser to sign in...'\n\
                 echo 'If it did not open, visit: https://claude.ai/oauth/authorize?state=abc'\n\
                 sleep 0.2\n\
                 echo 'Your long-lived token:'\n\
                 echo '{TOKEN}'\n\
                 echo 'Store it somewhere safe.'"
            ),
        );
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        let poll = run_to_end(&jobs, id);

        assert!(
            forge_config::claude_oauth_token().as_deref() == Some(TOKEN),
            "the token reached the secret store"
        );
        let json = serde_json::to_string(&poll).unwrap();
        assert!(
            !json.contains("sk-ant-oat"),
            "the token leaked into job events: {json}"
        );
        assert!(poll.events.iter().any(|e| matches!(
            e,
            JobEvent::Prompt { prompt: PromptKind::OpenUrl { url }, .. }
                if url == "https://claude.ai/oauth/authorize?state=abc"
        )));
        assert!(
            matches!(poll.events.last(), Some(JobEvent::Done { result: Some(ActionResult::Verified { source, .. }), .. }) if source == "claude-cli"),
            "ends with a verification: {:?}",
            poll.events.last()
        );
        forge_config::secret_store::delete(forge_config::CLAUDE_OAUTH_TOKEN_SLOT).unwrap();
    }

    #[test]
    fn when_the_token_cannot_be_read_the_flow_asks_for_a_paste() {
        let f = fixture();
        fake_claude(&f, "echo 'signed in, nothing printed'");
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        let poll = run_to_end(&jobs, id);
        assert!(matches!(
            poll.events.last(),
            Some(JobEvent::Done {
                result: Some(ActionResult::PasteNeeded {
                    slot: SecretSlot::ClaudeOauthToken
                }),
                ..
            })
        ));
        assert!(forge_config::claude_oauth_token().is_none());
    }

    #[test]
    fn a_code_the_page_shows_can_be_typed_back_and_is_never_logged() {
        let f = fixture();
        fake_claude(
            &f,
            &format!(
                "echo 'visit https://claude.ai/oauth/authorize?state=zzz'\n\
                 printf 'Paste code here if prompted > '\n\
                 read -r code\n\
                 echo \"got:$code\"\n\
                 echo '{TOKEN}'"
            ),
        );
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        let start = Instant::now();
        loop {
            let poll = jobs.poll(id, 0).unwrap();
            if poll.events.iter().any(|e| {
                matches!(
                    e,
                    JobEvent::Prompt {
                        prompt: PromptKind::Code { .. },
                        ..
                    }
                )
            }) {
                break;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "no code prompt: {:?}",
                poll.events
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(jobs.send_input(id, "SECRETCODE-12345"));
        let poll = run_to_end(&jobs, id);
        let json = serde_json::to_string(&poll).unwrap();
        assert!(
            !json.contains("SECRETCODE-12345"),
            "the typed code leaked: {json}"
        );
        assert!(!json.contains("sk-ant-oat"), "{json}");
        assert_eq!(forge_config::claude_oauth_token().as_deref(), Some(TOKEN));
        forge_config::secret_store::delete(forge_config::CLAUDE_OAUTH_TOKEN_SLOT).unwrap();
    }

    /// Claude Code's input box (Ink) reads raw chunks and treats a multi-byte chunk as pasted
    /// text: only a chunk that is exactly Enter submits. A code and "\r" in one write never does.
    #[test]
    fn a_typed_code_submits_in_an_ink_style_input_that_reads_raw_chunks() {
        let f = fixture();
        fake_claude(
            &f,
            &format!(
                "echo 'visit https://claude.ai/oauth/authorize?state=ink'\n\
                 stty raw -echo\n\
                 printf 'Paste code here if prompted > '\n\
                 code=''\n\
                 while :; do\n\
                   chunk=$(dd bs=4096 count=1 2>/dev/null)\n\
                   [ -z \"$chunk\" ] && exit 0\n\
                   [ \"$chunk\" = \"$(printf '\\r')\" ] && break\n\
                   code=\"$code$chunk\"\n\
                 done\n\
                 stty sane\n\
                 printf '\\r\\nreceived %s chars\\r\\n' \"${{#code}}\"\n\
                 echo '{TOKEN}'"
            ),
        );
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        let start = Instant::now();
        while !jobs.poll(id, 0).unwrap().events.iter().any(|e| {
            matches!(
                e,
                JobEvent::Prompt {
                    prompt: PromptKind::Code { .. },
                    ..
                }
            )
        }) {
            assert!(start.elapsed() < Duration::from_secs(10), "no code prompt");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(jobs.send_input(id, "CODE#STATE-123"));
        let poll = run_to_end(&jobs, id);
        let json = serde_json::to_string(&poll).unwrap();
        assert!(
            json.contains("received 14 chars"),
            "the code was not submitted: {json}"
        );
        assert_eq!(forge_config::claude_oauth_token().as_deref(), Some(TOKEN));
        forge_config::secret_store::delete(forge_config::CLAUDE_OAUTH_TOKEN_SLOT).unwrap();
    }

    #[test]
    fn a_missing_claude_is_a_plain_failure() {
        let _f = fixture();
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        let poll = run_to_end(&jobs, id);
        assert_eq!(poll.state, JobState::Failed);
    }

    #[test]
    fn cancelling_stops_the_child() {
        let f = fixture();
        fake_claude(&f, "echo waiting\nsleep 30");
        let jobs = Jobs::new();
        let id = jobs.run(crate::Action::ConnectClaude, Arc::new(NoHost));
        std::thread::sleep(Duration::from_millis(400));
        jobs.cancel(id);
        let poll = run_to_end(&jobs, id);
        assert_eq!(poll.state, JobState::Failed);
    }
}
