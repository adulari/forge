//! How long a bridge CLI gets to enumerate its models, when a recent answer is reused, and how the
//! CLIs are launched to ask: at low priority, in their own process group, in an empty directory and,
//! for `agy`, without the user's MCP servers (see [`agy_models_output`]).
//!
//! `agy models` is not a local lookup: it fetches the list over the network and takes 1.3-7 s
//! on a quiet machine (measured), longer while the daemon is starting under load or before DNS
//! works. A flat 5 s budget turned that into a failure on every daemon start.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use tokio::process::Command;

#[cfg(unix)]
use super::error_policy;
use super::{
    bridge_command, probe_failure_message, put_in_own_process_group, recall_bridge_models_at,
    BridgeModelSource, BridgeModels, CliKind,
};

/// The local CLIs answer from their own files; agy waits on the network.
const LOCAL_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const AGY_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// A successful `agy models` this recent is reused instead of re-run on every daemon restart.
const AGY_LIST_TTL_SECS: u64 = 60 * 60;

/// Niceness of every CLI a discovery pass starts. Discovery is never what a person is waiting for
/// (a pinned session does not route through it, an unpinned one reads the cache), so it yields to
/// the turn that is.
#[cfg(unix)]
const PROBE_NICE: libc::c_int = 10;

/// Where Antigravity keeps its sign-in, relative to the home directory. `agy models` needs nothing
/// else to answer; everything else under `~/.gemini` is the user's own agent configuration, which
/// `agy` loads (and starts) on every run, including an MCP server per `mcp_config.json` entry.
#[cfg(unix)]
const AGY_TOKEN: &str = ".gemini/antigravity-cli/antigravity-oauth-token";

impl CliKind {
    pub(super) fn probe_timeout(self) -> Duration {
        match self {
            CliKind::Antigravity => AGY_PROBE_TIMEOUT,
            CliKind::ClaudeCode | CliKind::Codex => LOCAL_PROBE_TIMEOUT,
        }
    }

    /// The list this bridge last advertised, if it is recent enough that asking again is wasted
    /// work. Only the networked probe has a TTL; the local ones are cheap to run every time.
    fn list_within_ttl(self, cached: Option<(Vec<String>, u64)>) -> Option<Vec<String>> {
        let ttl = match self {
            CliKind::Antigravity => AGY_LIST_TTL_SECS,
            CliKind::ClaudeCode | CliKind::Codex => return None,
        };
        cached
            .filter(|(_, age)| *age < ttl)
            .map(|(models, _)| models)
    }

    /// [`Self::bridge_models_detailed`] for discovery: reuses a recent successful list rather than
    /// paying the CLI's startup (and, for agy, a network round trip) on every daemon restart.
    /// `forge doctor` keeps probing live.
    pub async fn bridge_models_for_discovery(self) -> BridgeModels {
        let cached = super::bridge_model_cache_path()
            .and_then(|path| recall_bridge_models_at(&path, self.prefix(), super::now_unix()));
        match self.list_within_ttl(cached) {
            Some(models) => BridgeModels {
                models,
                source: BridgeModelSource::Live,
                probe_error: None,
            },
            None => self.bridge_models_detailed().await,
        }
    }
}

/// The one-line warning for a probe that failed, saying what the mesh uses instead.
pub(super) fn failure_warning(prefix: &str, error: &str, cached_age_secs: Option<u64>) -> String {
    match cached_age_secs {
        Some(age) => format!(
            "{prefix} model discovery failed: {error} — using the list it advertised {}m ago",
            age / 60
        ),
        None => format!(
            "{prefix} model discovery failed: {error} — the mesh will use an unverified model \
             list for this bridge"
        ),
    }
}

/// What every discovery child gets: a lower CPU priority, its own process group (so a timeout takes
/// the helpers it started down with it, see [`kill_group`]) and an empty working directory (so a CLI
/// that indexes or reads project files for its own purposes finds none of the user's).
pub(super) fn quiet_probe(cmd: &mut Command) {
    put_in_own_process_group(cmd);
    #[cfg(unix)]
    {
        // SAFETY: only `setpriority`, which is async-signal-safe, runs between fork and exec.
        unsafe {
            cmd.pre_exec(|| {
                libc::setpriority(libc::PRIO_PROCESS, 0, PROBE_NICE);
                Ok(())
            });
        }
    }
    if let Some(dir) = probe_cwd() {
        cmd.current_dir(dir);
    }
}

/// An empty directory under Forge's data dir, created once. `None` leaves the child in the caller's
/// directory, which is the old behaviour.
fn probe_cwd() -> Option<&'static Path> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = forge_config::data_dir()?.join("probe-cwd");
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    })
    .as_deref()
}

/// SIGKILL the process group of a child started with [`quiet_probe`] (its pid is the group id).
/// `kill_on_drop` alone ends only the child itself; the servers and helpers it started would
/// outlive a timed-out probe.
pub(super) fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid.and_then(|pid| libc::pid_t::try_from(pid).ok()) {
        // SAFETY: signalling a process group we created; a stale id fails with ESRCH.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Run a model-enumeration command and return its stdout, or a one-line explanation of why it
/// produced nothing. `home`, when given, replaces `HOME` for the child.
pub(super) async fn run_in(
    binary: &str,
    args: &[&str],
    timeout: Duration,
    home: Option<&Path>,
) -> Result<String, String> {
    let owned: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
    let label = format!("`{binary} {}`", args.join(" "));
    let mut cmd = bridge_command(binary, &owned);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    quiet_probe(&mut cmd);
    if let Some(home) = home {
        cmd.env("HOME", home);
    }
    let child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Err(format!("{label} could not be launched: {e}")),
    };
    let pid = child.id();
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Ok(Ok(out)) => Err(format!(
            "{label} {}: {}",
            match out.status.code() {
                Some(code) => format!("exited {code}"),
                None => "was killed by a signal".to_string(),
            },
            probe_failure_message(&out.stderr, &out.stdout)
        )),
        Ok(Err(e)) => Err(format!("{label} could not be launched: {e}")),
        Err(_) => {
            kill_group(pid);
            Err(format!("{label} timed out after {}s", timeout.as_secs()))
        }
    }
}

/// `agy models` without starting the user's agent configuration.
///
/// `agy` has no flag that skips its MCP servers, hooks or status line, and a plain `agy models`
/// starts every server in `~/.gemini/antigravity/mcp_config.json` (six `npx` servers on the
/// machine this was measured on: 1.07 GB of resident memory for a 1.8 s list) before it prints.
/// It does not need any of that, only its OAuth token, so it runs with a scratch `HOME` holding a
/// link to that one file (490 MB, no MCP servers).
///
/// If `agy` does not accept that home (it says it is signed out) the real environment gets the
/// question instead: the isolation must never be the reason a signed-in bridge is skipped.
pub(super) async fn agy_models_output(binary: &str, timeout: Duration) -> Result<String, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    agy_models_output_from(binary, timeout, home.as_deref()).await
}

async fn agy_models_output_from(
    binary: &str,
    timeout: Duration,
    real_home: Option<&Path>,
) -> Result<String, String> {
    let args = ["models"];
    #[cfg(unix)]
    if let Some(scratch) = real_home.and_then(ScratchHome::agy) {
        match run_in(binary, &args, timeout, Some(scratch.path())).await {
            Err(error) if error_policy::is_auth_failure(&error.to_ascii_lowercase()) => {
                tracing::debug!(
                    "agy refused the token-only probe home ({error}); asking with the real one"
                );
            }
            answered => return answered,
        }
    }
    #[cfg(not(unix))]
    let _ = real_home;
    run_in(binary, &args, timeout, None).await
}

/// A throwaway home directory, removed when dropped.
#[cfg(unix)]
struct ScratchHome(PathBuf);

#[cfg(unix)]
impl ScratchHome {
    /// A scratch home holding only agy's token, or `None` when the token is not where this was
    /// verified to be (any other layout is probed unisolated rather than guessed at).
    fn agy(real_home: &Path) -> Option<Self> {
        use std::os::unix::fs::{symlink, DirBuilderExt};

        let token = real_home.join(AGY_TOKEN);
        if !token.is_file() {
            return None;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let dir =
            std::env::temp_dir().join(format!("forge-agy-probe-{}-{nanos}", std::process::id()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).ok()?;
        let scratch = Self(dir);
        let link = scratch.0.join(AGY_TOKEN);
        std::fs::create_dir_all(link.parent()?).ok()?;
        symlink(&token, &link).ok()?;
        Some(scratch)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for ScratchHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cached(age: u64) -> Option<(Vec<String>, u64)> {
        Some((vec!["gemini-3.7-flash-low".to_string()], age))
    }

    #[test]
    fn agy_gets_a_budget_that_fits_its_measured_worst_case() {
        assert!(CliKind::Antigravity.probe_timeout() >= Duration::from_secs(15));
        assert_eq!(CliKind::Codex.probe_timeout(), Duration::from_secs(5));
    }

    #[test]
    fn a_recent_agy_list_is_reused_and_an_old_one_is_not() {
        assert!(CliKind::Antigravity.list_within_ttl(cached(60)).is_some());
        assert!(CliKind::Antigravity
            .list_within_ttl(cached(AGY_LIST_TTL_SECS))
            .is_none());
        assert!(CliKind::Antigravity.list_within_ttl(None).is_none());
    }

    #[test]
    fn local_bridges_are_always_probed_live() {
        assert!(CliKind::Codex.list_within_ttl(cached(1)).is_none());
        assert!(CliKind::ClaudeCode.list_within_ttl(cached(1)).is_none());
    }

    #[test]
    fn the_warning_says_whether_a_cached_list_stands_in() {
        assert!(failure_warning("agy-cli", "timed out", Some(600)).contains("10m ago"));
        assert!(failure_warning("agy-cli", "timed out", None).contains("unverified"));
    }

    #[cfg(unix)]
    mod launched {
        use super::*;
        use crate::cli_provider::tests::install_fake_cli;

        const SIGN_IN: &str = "Please sign in to view available models.";

        /// A real home holding a token and an MCP configuration, plus a directory the fake `agy`
        /// writes what it saw into.
        fn homes() -> (tempfile::TempDir, tempfile::TempDir) {
            let home = tempfile::tempdir().unwrap();
            let token = home.path().join(AGY_TOKEN);
            std::fs::create_dir_all(token.parent().unwrap()).unwrap();
            std::fs::write(&token, "token").unwrap();
            let mcp = home.path().join(".gemini/antigravity/mcp_config.json");
            std::fs::create_dir_all(mcp.parent().unwrap()).unwrap();
            std::fs::write(&mcp, "{}").unwrap();
            (home, tempfile::tempdir().unwrap())
        }

        #[tokio::test]
        async fn agy_is_asked_from_a_home_that_holds_only_its_token() {
            let (home, seen) = homes();
            let agy = install_fake_cli(
                "agy-isolated",
                &format!(
                    r#"#!/bin/sh
[ "$1" = models ] || exit 2
ls -A "$HOME/.gemini" > "{seen}/gemini"
echo "$HOME" > "{seen}/home"
[ -e "$HOME/.gemini/antigravity-cli/antigravity-oauth-token" ] || {{ echo '{SIGN_IN}' >&2; exit 1; }}
printf 'gemini-9-test-low\tGemini 9 Test (Low)\n'
"#,
                    seen = seen.path().display()
                ),
            );

            let out = agy_models_output_from(&agy, Duration::from_secs(10), Some(home.path()))
                .await
                .unwrap();

            assert!(out.contains("gemini-9-test-low"));
            let scratch = std::fs::read_to_string(seen.path().join("home")).unwrap();
            let scratch = Path::new(scratch.trim());
            assert_ne!(scratch, home.path(), "the real home must not be agy's HOME");
            assert_eq!(
                std::fs::read_to_string(seen.path().join("gemini"))
                    .unwrap()
                    .trim(),
                "antigravity-cli",
                "agy saw its token and nothing of the user's MCP configuration"
            );
            assert!(!scratch.exists(), "the scratch home is removed afterwards");
            assert!(
                home.path().join(AGY_TOKEN).is_file(),
                "the real token is untouched"
            );
        }

        #[test]
        fn a_home_without_agys_token_gets_no_scratch_home() {
            let empty = tempfile::tempdir().unwrap();
            assert!(ScratchHome::agy(empty.path()).is_none());
        }

        #[tokio::test]
        async fn a_scratch_home_agy_refuses_falls_back_to_the_real_environment() {
            let (home, seen) = homes();
            let agy = install_fake_cli(
                "agy-refuses-once",
                &format!(
                    r#"#!/bin/sh
echo x >> "{seen}/calls"
[ "$(wc -l < "{seen}/calls")" -gt 1 ] || {{ echo '{SIGN_IN}' >&2; exit 1; }}
printf 'gemini-9-test-low\tGemini 9 Test (Low)\n'
"#,
                    seen = seen.path().display()
                ),
            );

            let out = agy_models_output_from(&agy, Duration::from_secs(10), Some(home.path()))
                .await
                .unwrap();

            assert!(out.contains("gemini-9-test-low"));
            let calls = std::fs::read_to_string(seen.path().join("calls")).unwrap();
            assert_eq!(calls.lines().count(), 2, "one isolated try, one real one");
        }

        #[tokio::test]
        async fn a_genuinely_signed_out_agy_is_still_reported_as_signed_out() {
            let (home, seen) = homes();
            let agy = install_fake_cli(
                "agy-signed-out",
                &format!("#!/bin/sh\necho '{SIGN_IN}' >&2\nexit 1\n"),
            );
            let _ = seen;

            let error = agy_models_output_from(&agy, Duration::from_secs(10), Some(home.path()))
                .await
                .unwrap_err();

            assert!(
                error_policy::is_auth_failure(&error.to_ascii_lowercase()),
                "{error}"
            );
        }

        #[cfg(target_os = "linux")]
        #[tokio::test]
        async fn a_timed_out_probe_takes_the_helpers_it_started_with_it() {
            let seen = tempfile::tempdir().unwrap();
            let cli = install_fake_cli(
                "probe-with-helper",
                &format!(
                    "#!/bin/sh\nsleep 60 &\necho $! > \"{pid}\"\nwait\n",
                    pid = seen.path().join("helper.pid").display()
                ),
            );

            let error = run_in(&cli, &[], Duration::from_millis(400), None)
                .await
                .unwrap_err();

            assert!(error.contains("timed out"), "{error}");
            let helper: u32 = std::fs::read_to_string(seen.path().join("helper.pid"))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            let gone = || {
                std::fs::read_to_string(format!("/proc/{helper}/stat")).map_or(true, |stat| {
                    stat.rsplit(") ").next().is_some_and(|r| r.starts_with('Z'))
                })
            };
            for _ in 0..100 {
                if gone() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            panic!("the helper (pid {helper}) outlived its timed-out probe");
        }

        #[tokio::test]
        async fn a_probe_runs_niced_in_an_empty_directory() {
            let cli = install_fake_cli(
                "probe-env",
                "#!/bin/sh\nps -o ni= -p $$ | tr -d ' '\npwd -P\n",
            );

            let out = run_in(&cli, &[], Duration::from_secs(10), None)
                .await
                .unwrap();

            let mut lines = out.lines();
            let nice: i32 = lines.next().unwrap().parse().unwrap();
            assert!(nice >= PROBE_NICE, "niceness {nice}");
            if let Some(dir) = probe_cwd() {
                let cwd = lines.next().unwrap();
                assert_eq!(Path::new(cwd), dir.canonicalize().unwrap());
                assert_eq!(
                    std::fs::read_dir(dir).unwrap().count(),
                    0,
                    "the probe directory stays empty"
                );
            }
        }
    }
}
