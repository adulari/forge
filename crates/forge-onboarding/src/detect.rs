//! Machine detection. Cheap facts are read inline; anything that can block (a subprocess, the OS
//! keyring, a socket, the host's service manager) runs on its own thread, and the whole pass
//! stops waiting at the budget. A probe that misses it leaves its facts at the empty default and
//! is listed in `Machine::timed_out`; detection never blocks a front end for longer than that.

use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_provider::CliKind;

use crate::host::Host;
use crate::machine::*;
use crate::proc::{run_capture, version_of};
use crate::providers;

/// How long detection waits for slow probes.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(1500);

/// The secret slot holding a dedicated Claude token for the bridge.
pub const CLAUDE_TOKEN_SLOT: &str = "claude_oauth_token";

type Patch = Box<dyn FnOnce(&mut Machine) + Send>;

pub fn detect(host: Arc<dyn Host>) -> Machine {
    detect_with(host, DEFAULT_BUDGET)
}

pub fn detect_with(host: Arc<dyn Host>, budget: Duration) -> Machine {
    let started = Instant::now();
    let mut machine = base_machine(host.as_ref());

    let (tx, rx) = mpsc::channel::<(&'static str, Patch)>();
    let mut pending: Vec<&'static str> = Vec::new();
    let mut spawn = |name: &'static str, f: Box<dyn FnOnce() -> Patch + Send>| {
        pending.push(name);
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send((name, f()));
        });
    };

    spawn("keys", Box::new(keys_probe));
    spawn("oauth", Box::new(oauth_probe));
    spawn("bridge_versions", Box::new(bridge_versions_probe));
    spawn("imports", Box::new(imports_probe));
    spawn("git", Box::new(git_probe));
    spawn("docker", Box::new(docker_probe));
    {
        let host = host.clone();
        spawn("local", Box::new(move || local_probe(host)));
    }
    {
        let host = host.clone();
        spawn("service", Box::new(move || service_probe(host)));
    }
    {
        let host = host.clone();
        spawn("anywhere", Box::new(move || anywhere_probe(host)));
    }
    drop(tx);

    let deadline = started + budget;
    while !pending.is_empty() {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok((name, patch)) => {
                patch(&mut machine);
                pending.retain(|p| *p != name);
            }
            Err(_) => break,
        }
    }
    machine.timed_out = pending.iter().map(|p| p.to_string()).collect();
    machine.took_ms = started.elapsed().as_millis() as u64;
    machine
}

/// Facts that need no subprocess and no keyring: filesystem and environment only.
fn base_machine(host: &dyn Host) -> Machine {
    let exe = host.forge_exe();
    let cfg = forge_config::load().ok();
    let settings = cfg
        .as_ref()
        .map(|c| SettingsFacts {
            permission: c.permission_mode.key().to_string(),
            credit_mode: format!("{:?}", c.mesh.credit_mode).to_lowercase(),
            keybind_preset: c.keybinds.preset.as_str().to_string(),
            notifications: c.notifications.enabled,
            theme: c.tui.theme.clone(),
            reporting: crate::reporting::mode(),
            reporting_chosen: crate::reporting::chosen(),
        })
        .unwrap_or_else(|| SettingsFacts {
            permission: "default".into(),
            credit_mode: "normal".into(),
            keybind_preset: "forge".into(),
            notifications: false,
            theme: "auto".into(),
            reporting: "auto".into(),
            reporting_chosen: false,
        });
    let keybinds_active = settings.keybind_preset == "claude-code";
    let cwd = std::env::current_dir().unwrap_or_default();
    Machine {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        terminal: terminal_facts(),
        forge: ForgeInstall {
            installed: exe.is_some(),
            version: host.forge_version(),
            exe: exe.map(|p| p.display().to_string()),
        },
        bridges: bridge_facts(),
        keys: key_facts_unchecked(),
        local: LocalFacts::default(),
        search: SearchFacts {
            brave_key: false,
            searxng_running: searxng_listening(),
            docker: false,
        },
        benchmarks: BenchFacts::default(),
        imports: ImportFacts {
            keybinds_active,
            ..import_dirs()
        },
        settings,
        desktop: DesktopFacts {
            supported: matches!(std::env::consts::OS, "linux" | "macos" | "windows"),
            installed: desktop_installed(),
        },
        project: ProjectFacts {
            git_repo: in_git_repo(&cwd),
            cwd: cwd.display().to_string(),
        },
        ..Machine::default()
    }
}

fn terminal_facts() -> TerminalFacts {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let dumb = var("TERM") == "dumb";
    let colorterm = var("COLORTERM").to_lowercase();
    let locale = format!("{}{}", var("LC_ALL"), var("LANG")).to_lowercase();
    TerminalFacts {
        color: !dumb && std::env::var_os("NO_COLOR").is_none(),
        truecolor: colorterm.contains("truecolor") || colorterm.contains("24bit"),
        unicode: locale.contains("utf-8") || locale.contains("utf8"),
    }
}

/// Install state and login of every bridge CLI, without launching any of them.
fn bridge_facts() -> Vec<BridgeFacts> {
    CliKind::all()
        .into_iter()
        .map(|kind| {
            let installed = kind.available();
            let login = if !installed {
                Login::Unknown
            } else {
                match forge_provider::bridge_credentials(kind) {
                    forge_provider::CliCredentials::Present => Login::SignedIn,
                    forge_provider::CliCredentials::Absent => Login::SignedOut,
                    forge_provider::CliCredentials::Unknown => Login::Unknown,
                }
            };
            let (name, binary) = match kind {
                CliKind::ClaudeCode => ("Claude Code", "claude"),
                CliKind::Codex => ("Codex", "codex"),
                CliKind::Antigravity => ("Antigravity", "agy"),
            };
            BridgeFacts {
                id: kind.prefix().to_string(),
                name: name.to_string(),
                binary: binary.to_string(),
                installed,
                version: None,
                login,
                token_stored: false,
            }
        })
        .collect()
}

/// Every key provider Forge knows, all marked unconfigured until the keyring probe answers.
fn key_facts_unchecked() -> Vec<KeyFacts> {
    forge_config::known_key_providers()
        .map(|id| {
            let feat = providers::featured(id);
            KeyFacts {
                id: id.to_string(),
                name: providers::display_name(id),
                configured: false,
                from_env: false,
                free_tier: feat.is_some_and(|f| f.free_tier),
                key_url: feat.map(|f| f.key_url.to_string()),
                featured: feat.is_some(),
            }
        })
        .collect()
}

fn keys_probe() -> Patch {
    let configured: Vec<(String, bool, bool)> = forge_config::known_key_providers()
        .map(|id| {
            (
                id.to_string(),
                forge_config::has_api_key(id),
                forge_config::provider_environment_key_present(id),
            )
        })
        .collect();
    let brave = forge_config::has_search_key("brave");
    let bench = forge_config::benchmark_api_key().is_some();
    let claude_token = claude_token_stored();
    Box::new(move |m| {
        for key in &mut m.keys {
            if let Some((_, has, env)) = configured.iter().find(|(id, _, _)| *id == key.id) {
                key.configured = *has;
                key.from_env = *has && *env;
            }
        }
        m.search.brave_key = brave;
        m.benchmarks.own_key = bench;
        for b in &mut m.bridges {
            if b.id == "claude-cli" {
                b.token_stored = claude_token;
            }
        }
    })
}

/// Whether a dedicated Forge token for the Claude bridge is stored. Adapter: the slot name is
/// the one the bridge credential change introduces; swap this body for
/// `forge_config::claude_oauth_token()` once that lands.
pub fn claude_token_stored() -> bool {
    std::env::var("CLAUDE_CODE_OAUTH_TOKEN").is_ok_and(|v| !v.trim().is_empty())
        || forge_config::secret_store::get(CLAUDE_TOKEN_SLOT).is_some_and(|v| !v.is_empty())
}

fn oauth_probe() -> Patch {
    let codex = forge_provider::has_codex_oauth_session();
    let xai = forge_provider::has_xai_oauth_session();
    Box::new(move |m| {
        m.oauth = vec![
            OauthFacts {
                id: "codex-oauth".into(),
                name: "ChatGPT subscription".into(),
                signed_in: codex,
            },
            OauthFacts {
                id: "xai-oauth".into(),
                name: "SuperGrok subscription".into(),
                signed_in: xai,
            },
        ];
    })
}

fn bridge_versions_probe() -> Patch {
    let versions: Vec<(String, Option<String>)> = CliKind::all()
        .into_iter()
        .filter(|k| k.available())
        .map(|kind| {
            let binary = match kind {
                CliKind::ClaudeCode => "claude",
                CliKind::Codex => "codex",
                CliKind::Antigravity => "agy",
            };
            let found = which(binary).or_else(|| well_known(binary));
            let version = found
                .as_deref()
                .and_then(|p| version_of(&p.display().to_string(), Duration::from_millis(1200)));
            (kind.prefix().to_string(), version)
        })
        .collect();
    Box::new(move |m| {
        for (id, version) in versions {
            if let Some(b) = m.bridges.iter_mut().find(|b| b.id == id) {
                b.version = version;
            }
        }
    })
}

fn which(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|p| p.is_file())
}

fn well_known(binary: &str) -> Option<PathBuf> {
    let home = forge_config::home_dir()?;
    [
        home.join(".local/bin").join(binary),
        home.join(".claude/local").join(binary),
        home.join(".npm-global/bin").join(binary),
        PathBuf::from("/usr/local/bin").join(binary),
        PathBuf::from("/opt/homebrew/bin").join(binary),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

fn import_dirs() -> ImportFacts {
    ImportFacts {
        claude_dir: forge_config::claude_dir().is_some_and(|d| d.is_dir()),
        codex_dir: forge_config::codex_dir().is_some_and(|d| d.is_dir()),
        cursor_dir: forge_config::cursor_dir().is_some_and(|d| d.is_dir()),
        ..ImportFacts::default()
    }
}

fn imports_probe() -> Patch {
    let claude = forge_config::claude_dir();
    let commands = claude
        .as_deref()
        .map(|d| count_files(&d.join("commands"), "md"))
        .unwrap_or(0);
    let skills = claude
        .as_deref()
        .map(|d| count_dirs(&d.join("skills")))
        .unwrap_or(0);
    let cwd = std::env::current_dir().unwrap_or_default();
    let mcp: usize = forge_config::discover_import_sources(&cwd)
        .iter()
        .map(|s| s.servers.len())
        .sum();
    Box::new(move |m| {
        m.imports.commands = commands;
        m.imports.skills = skills;
        m.imports.mcp_servers = mcp;
    })
}

fn count_files(dir: &Path, ext: &str) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|x| x == ext))
                .count()
        })
        .unwrap_or(0)
}

fn count_dirs(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.path().is_dir())
                .count()
        })
        .unwrap_or(0)
}

fn git_probe() -> Patch {
    let get = |key: &str| {
        run_capture("git", &["config", "--get", key], Duration::from_millis(800))
            .filter(|(ok, _)| *ok)
            .map(|(_, out)| out.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let (name, email) = (get("user.name"), get("user.email"));
    Box::new(move |m| m.git = GitFacts { name, email })
}

fn docker_probe() -> Patch {
    let up = run_capture(
        "docker",
        &["info", "--format", "{{.ServerVersion}}"],
        Duration::from_millis(1200),
    )
    .is_some_and(|(ok, _)| ok);
    Box::new(move |m| m.search.docker = up)
}

fn searxng_listening() -> bool {
    TcpStream::connect_timeout(&([127, 0, 0, 1], 8888).into(), Duration::from_millis(150)).is_ok()
}

fn local_probe(host: Arc<dyn Host>) -> Patch {
    let local = host.local();
    Box::new(move |m| m.local = local)
}

fn service_probe(host: Arc<dyn Host>) -> Patch {
    let service = host.service();
    Box::new(move |m| m.service = service)
}

fn anywhere_probe(host: Arc<dyn Host>) -> Patch {
    let anywhere = host.anywhere();
    Box::new(move |m| m.anywhere = anywhere)
}

fn in_git_repo(start: &Path) -> bool {
    start.ancestors().any(|dir| dir.join(".git").exists())
}

/// Whether the Forge desktop app is installed, from where the installers put it.
pub fn desktop_installed() -> bool {
    let Some(home) = forge_config::home_dir() else {
        return false;
    };
    let candidates: Vec<PathBuf> = match std::env::consts::OS {
        "macos" => vec![
            PathBuf::from("/Applications/Forge.app"),
            home.join("Applications/Forge.app"),
        ],
        "windows" => std::env::var_os("LOCALAPPDATA")
            .map(|d| {
                vec![PathBuf::from(d)
                    .join("Programs")
                    .join("Forge")
                    .join("Forge.exe")]
            })
            .unwrap_or_default(),
        _ => {
            let data = std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"));
            vec![
                home.join(".local/bin/forge-desktop"),
                home.join(".local/bin/forge-desktop.AppImage"),
                data.join("applications/forge-desktop.desktop"),
            ]
        }
    };
    candidates.iter().any(|p| p.exists())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::host::NoHost;
    use crate::test_support::{env_lock, ScopedEnv};
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _guard: std::sync::MutexGuard<'static, ()>,
        _env: ScopedEnv,
        home: tempfile::TempDir,
    }

    /// A fake HOME with isolated XDG dirs, an empty PATH-only bin dir and no provider keys.
    fn fixture() -> Fixture {
        let guard = env_lock();
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let mut vars: Vec<(&str, Option<&OsStr>)> = vec![
            ("HOME", Some(home.path().as_os_str())),
            ("XDG_CONFIG_HOME", None),
            ("XDG_DATA_HOME", None),
            ("CLAUDE_CONFIG_DIR", None),
            ("CODEX_HOME", None),
            ("ANTHROPIC_API_KEY", None),
            ("CLAUDE_CODE_OAUTH_TOKEN", None),
            ("OPENAI_API_KEY", None),
            ("GROQ_API_KEY", None),
            ("FORGE_DB", None),
            ("COLORTERM", None),
            ("NO_COLOR", None),
            ("LANG", Some(OsStr::new("en_US.UTF-8"))),
        ];
        let path = bin.as_os_str().to_owned();
        vars.push(("PATH", Some(path.as_os_str())));
        let env = ScopedEnv::set(&vars);
        Fixture {
            _guard: guard,
            _env: env,
            home,
        }
    }

    fn fake_cli(f: &Fixture, name: &str, version_line: &str) {
        let path = f.home.path().join("bin").join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho '{version_line}'\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn detect_fast() -> Machine {
        detect_with(Arc::new(NoHost), Duration::from_secs(5))
    }

    #[test]
    fn a_signed_in_claude_reports_its_version_and_login() {
        let f = fixture();
        fake_cli(&f, "claude", "2.1.4 (Claude Code)");
        std::fs::create_dir_all(f.home.path().join(".claude")).unwrap();
        std::fs::write(f.home.path().join(".claude/.credentials.json"), "{}").unwrap();
        let machine = detect_fast();
        let claude = machine
            .bridges
            .iter()
            .find(|b| b.id == "claude-cli")
            .unwrap();
        assert!(claude.installed);
        assert_eq!(claude.login, Login::SignedIn);
        assert_eq!(claude.version.as_deref(), Some("2.1.4"));
        let card = machine.model_sources().remove(0);
        assert_eq!(card.detail, "Claude Code 2.1.4 · signed in");
    }

    #[test]
    fn an_installed_but_signed_out_claude_needs_a_login() {
        let f = fixture();
        fake_cli(&f, "claude", "2.1.4");
        let machine = detect_fast();
        let claude = machine
            .bridges
            .iter()
            .find(|b| b.id == "claude-cli")
            .unwrap();
        assert!(claude.installed);
        if !cfg!(target_os = "macos") {
            assert_eq!(claude.login, Login::SignedOut);
            assert!(machine.ready_sources().is_empty());
        }
    }

    #[test]
    fn codex_is_signed_in_by_its_auth_file_and_absent_without_one() {
        let f = fixture();
        fake_cli(&f, "codex", "codex-cli 0.46.0");
        let machine = detect_fast();
        let codex = machine
            .bridges
            .iter()
            .find(|b| b.id == "codex-cli")
            .unwrap();
        assert_eq!(codex.login, Login::SignedOut);

        std::fs::create_dir_all(f.home.path().join(".codex")).unwrap();
        std::fs::write(f.home.path().join(".codex/auth.json"), "{}").unwrap();
        let machine = detect_fast();
        let codex = machine
            .bridges
            .iter()
            .find(|b| b.id == "codex-cli")
            .unwrap();
        assert_eq!(codex.login, Login::SignedIn);
        assert_eq!(codex.version.as_deref(), Some("0.46.0"));
    }

    #[test]
    fn a_machine_with_nothing_installed_reports_every_bridge_missing() {
        let _f = fixture();
        let machine = detect_fast();
        assert_eq!(machine.bridges.len(), 3);
        assert!(machine.bridges.iter().all(|b| !b.installed));
        assert!(machine.ready_sources().is_empty());
        assert!(machine.timed_out.is_empty());
    }

    #[test]
    fn keys_are_found_in_the_environment_and_in_the_keyring() {
        let _f = fixture();
        std::env::set_var("GROQ_API_KEY", "gsk-test");
        forge_config::store_api_key("gemini", "AIza-test").unwrap();
        let machine = detect_fast();
        let groq = machine.keys.iter().find(|k| k.id == "groq").unwrap();
        assert!(groq.configured && groq.from_env && groq.free_tier);
        let gemini = machine.keys.iter().find(|k| k.id == "gemini").unwrap();
        assert!(gemini.configured && !gemini.from_env);
        let anthropic = machine.keys.iter().find(|k| k.id == "anthropic").unwrap();
        assert!(!anthropic.configured);
        let details: Vec<String> = machine
            .ready_sources()
            .into_iter()
            .map(|s| s.detail)
            .collect();
        assert!(details.contains(&"API key found in your environment".to_string()));
        forge_config::remove_api_key("gemini").unwrap();
    }

    #[test]
    fn the_claude_token_slot_is_reported_without_its_value() {
        let f = fixture();
        fake_cli(&f, "claude", "2.1.4");
        forge_config::secret_store::set(CLAUDE_TOKEN_SLOT, "sk-ant-oat01-secret").unwrap();
        let machine = detect_fast();
        let json = serde_json::to_string(&machine).unwrap();
        assert!(!json.contains("sk-ant-oat01-secret"));
        let claude = machine
            .bridges
            .iter()
            .find(|b| b.id == "claude-cli")
            .unwrap();
        assert!(claude.token_stored);
        forge_config::secret_store::delete(CLAUDE_TOKEN_SLOT).unwrap();
    }

    #[test]
    fn importable_settings_are_counted_from_the_claude_directory() {
        let f = fixture();
        let claude = f.home.path().join(".claude");
        std::fs::create_dir_all(claude.join("commands")).unwrap();
        std::fs::create_dir_all(claude.join("skills/review")).unwrap();
        std::fs::create_dir_all(claude.join("skills/deploy")).unwrap();
        std::fs::write(claude.join("commands/a.md"), "x").unwrap();
        std::fs::write(claude.join("commands/b.md"), "x").unwrap();
        std::fs::write(claude.join("commands/notes.txt"), "x").unwrap();
        let machine = detect_fast();
        assert!(machine.imports.claude_dir);
        assert_eq!(machine.imports.commands, 2);
        assert_eq!(machine.imports.skills, 2);
        assert!(machine.imports.anything());
    }

    #[test]
    fn a_clean_home_has_nothing_to_import() {
        let _f = fixture();
        let machine = detect_fast();
        assert!(!machine.imports.anything());
    }

    #[test]
    fn host_facts_are_merged_in() {
        struct Local;
        impl Host for Local {
            fn local(&self) -> LocalFacts {
                LocalFacts {
                    ollama_installed: true,
                    ollama_serving: true,
                    models: vec!["qwen2.5-coder:7b".into()],
                    memory_gb: 16.0,
                    recommended: None,
                }
            }
            fn service(&self) -> ServiceFacts {
                ServiceFacts {
                    supported: true,
                    installed: true,
                    running: false,
                }
            }
        }
        let _f = fixture();
        let machine = detect_with(Arc::new(Local), Duration::from_secs(5));
        assert_eq!(machine.local.models, ["qwen2.5-coder:7b"]);
        assert!(machine.service.installed && !machine.service.running);
        assert_eq!(machine.model_sources()[0].id, "ollama");
    }

    #[test]
    fn a_slow_probe_is_abandoned_at_the_budget_and_reported() {
        struct Slow;
        impl Host for Slow {
            fn service(&self) -> ServiceFacts {
                std::thread::sleep(Duration::from_millis(1500));
                ServiceFacts {
                    supported: true,
                    installed: true,
                    running: true,
                }
            }
        }
        let _f = fixture();
        let start = Instant::now();
        let machine = detect_with(Arc::new(Slow), Duration::from_millis(400));
        assert!(start.elapsed() < Duration::from_millis(1400));
        assert_eq!(machine.timed_out, ["service"]);
        assert!(
            !machine.service.installed,
            "a missed probe leaves the default"
        );
    }

    #[test]
    fn terminal_capabilities_come_from_the_environment() {
        let _f = fixture();
        std::env::set_var("COLORTERM", "truecolor");
        let machine = detect_fast();
        assert!(machine.terminal.truecolor && machine.terminal.unicode && machine.terminal.color);
        std::env::set_var("NO_COLOR", "1");
        assert!(!detect_fast().terminal.color);
    }
}
