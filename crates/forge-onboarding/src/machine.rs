//! What detection found on this machine, as plain serializable data. Front ends render it; the
//! daemon serves it as JSON. Nothing in here is a secret: keys are reported as present or absent.

use serde::{Deserialize, Serialize};

/// Everything the onboarding knows about the machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Machine {
    pub os: String,
    pub arch: String,
    pub terminal: TerminalFacts,
    pub forge: ForgeInstall,
    pub bridges: Vec<BridgeFacts>,
    pub oauth: Vec<OauthFacts>,
    /// Every key-based provider Forge can talk to, configured or not.
    pub keys: Vec<KeyFacts>,
    pub local: LocalFacts,
    pub search: SearchFacts,
    pub benchmarks: BenchFacts,
    pub imports: ImportFacts,
    pub settings: SettingsFacts,
    pub service: ServiceFacts,
    pub anywhere: AnywhereFacts,
    pub desktop: DesktopFacts,
    pub git: GitFacts,
    pub project: ProjectFacts,
    /// Probes that did not answer inside the budget. Their facts stay at the empty default.
    pub timed_out: Vec<String>,
    pub took_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalFacts {
    pub color: bool,
    pub truecolor: bool,
    pub unicode: bool,
}

/// The `forge` binary itself. A terminal run is always installed; the desktop app checks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ForgeInstall {
    pub installed: bool,
    pub version: String,
    pub exe: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Login {
    SignedIn,
    SignedOut,
    /// No evidence either way (the CLI keeps its login somewhere Forge cannot read).
    #[default]
    Unknown,
}

/// A subscription CLI Forge can drive: Claude Code, Codex or Antigravity.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BridgeFacts {
    /// Bridge prefix: `claude-cli`, `codex-cli` or `agy-cli`.
    pub id: String,
    pub name: String,
    pub binary: String,
    pub installed: bool,
    pub version: Option<String>,
    pub login: Login,
    /// A dedicated Forge token is stored for this bridge (Claude only).
    pub token_stored: bool,
    /// The CLI has its own interactive login on this machine (Claude only). The bridge no
    /// longer shares it, so such a machine needs a one-click reconnect.
    pub main_login: bool,
}

/// A subscription Forge signs in to itself, without a CLI.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OauthFacts {
    pub id: String,
    pub name: String,
    pub signed_in: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyFacts {
    pub id: String,
    pub name: String,
    pub configured: bool,
    /// The key comes from an environment variable rather than the keyring.
    pub from_env: bool,
    pub free_tier: bool,
    pub key_url: Option<String>,
    /// One of the providers the model step leads with.
    pub featured: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalRecommendation {
    pub tag: String,
    pub label: String,
    pub memory_gb: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalFacts {
    pub ollama_installed: bool,
    pub ollama_serving: bool,
    pub models: Vec<String>,
    /// Memory a local model can use, in GiB.
    pub memory_gb: f64,
    pub recommended: Option<LocalRecommendation>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchFacts {
    pub brave_key: bool,
    pub searxng_running: bool,
    pub docker: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchFacts {
    /// A personal Artificial Analysis key is set and overrides the shared data.
    pub own_key: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportFacts {
    pub claude_dir: bool,
    pub codex_dir: bool,
    pub cursor_dir: bool,
    pub commands: usize,
    pub skills: usize,
    pub mcp_servers: usize,
    pub keybinds_active: bool,
}

impl ImportFacts {
    /// Whether the step has anything to offer.
    pub fn anything(&self) -> bool {
        self.claude_dir || self.codex_dir || self.cursor_dir || self.mcp_servers > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsFacts {
    /// `PermissionMode::key()`.
    pub permission: String,
    /// `snake_case` credit mode.
    pub credit_mode: String,
    /// `forge` or `claude-code`.
    pub keybind_preset: String,
    pub notifications: bool,
    /// Effective error-report mode: `auto`, `ask` or `off`.
    pub reporting: String,
    /// The person has picked a mode (it is written in the config).
    pub reporting_chosen: bool,
    /// `auto`, `dark` or `light`.
    pub theme: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceFacts {
    pub supported: bool,
    pub installed: bool,
    pub running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnywhereFacts {
    pub signed_in: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopFacts {
    pub supported: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GitFacts {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectFacts {
    pub cwd: String,
    pub git_repo: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    ClaudeCode,
    Codex,
    Antigravity,
    CodexOauth,
    XaiOauth,
    ApiKey,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// Found and usable: a login, a key or a pulled model.
    Ready,
    /// The tool is installed but not signed in.
    NeedsLogin,
    /// Nothing there yet; the user can paste a key or sign in.
    NeedsSetup,
    /// The tool is not installed.
    NotInstalled,
}

/// One card on the model step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSource {
    /// The id verification and the state file use (`claude-cli`, `groq`, `ollama`).
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub state: SourceState,
    /// What was found, in one line: `Claude Code 2.1.4 · signed in`.
    pub detail: String,
    pub free_tier: bool,
    pub key_url: Option<String>,
}

impl Machine {
    /// The model cards, best first: what already works, then what is one step away, then the
    /// rest. Pure, so the order is testable without a terminal.
    pub fn model_sources(&self) -> Vec<ModelSource> {
        let mut ready = Vec::new();
        let mut login = Vec::new();
        let mut subs = Vec::new();
        let mut free = Vec::new();
        let mut local = Vec::new();
        let mut paid = Vec::new();
        let mut missing = Vec::new();

        for b in &self.bridges {
            let kind = match b.id.as_str() {
                "claude-cli" => SourceKind::ClaudeCode,
                "codex-cli" => SourceKind::Codex,
                _ => SourceKind::Antigravity,
            };
            let version = b
                .version
                .as_deref()
                .map(|v| format!(" {v}"))
                .unwrap_or_default();
            let mut card = ModelSource {
                id: b.id.clone(),
                name: b.name.clone(),
                kind,
                state: SourceState::NotInstalled,
                detail: format!("{} is not installed", b.name),
                free_tier: false,
                key_url: None,
            };
            if b.installed {
                match b.login {
                    Login::SignedOut => {
                        card.state = SourceState::NeedsLogin;
                        card.detail = if b.id == "claude-cli" && b.main_login {
                            format!(
                                "Reconnect {} · one click, keeps your main Claude login safe",
                                b.name
                            )
                        } else {
                            format!("{}{version} · installed, not signed in", b.name)
                        };
                        login.push(card);
                    }
                    Login::SignedIn | Login::Unknown => {
                        card.state = SourceState::Ready;
                        let status = if b.login == Login::SignedIn {
                            "signed in"
                        } else {
                            "installed"
                        };
                        let token = if b.token_stored {
                            " · Forge token set"
                        } else {
                            ""
                        };
                        card.detail = format!("{}{version} · {status}{token}", b.name);
                        ready.push(card);
                    }
                }
            } else {
                missing.push(card);
            }
        }

        for o in &self.oauth {
            let kind = if o.id == "codex-oauth" {
                SourceKind::CodexOauth
            } else {
                SourceKind::XaiOauth
            };
            let card = ModelSource {
                id: o.id.clone(),
                name: o.name.clone(),
                kind,
                state: if o.signed_in {
                    SourceState::Ready
                } else {
                    SourceState::NeedsSetup
                },
                detail: if o.signed_in {
                    format!("{} · signed in", o.name)
                } else {
                    "Sign in with your subscription, no API key".to_string()
                },
                free_tier: false,
                key_url: None,
            };
            if o.signed_in {
                ready.push(card);
            } else {
                subs.push(card);
            }
        }

        for k in &self.keys {
            if !k.configured && !k.featured {
                continue;
            }
            let card = ModelSource {
                id: k.id.clone(),
                name: k.name.clone(),
                kind: SourceKind::ApiKey,
                state: if k.configured {
                    SourceState::Ready
                } else {
                    SourceState::NeedsSetup
                },
                detail: if k.configured && k.from_env {
                    "API key found in your environment".to_string()
                } else if k.configured {
                    "API key stored".to_string()
                } else if k.free_tier {
                    "Free tier, paste a key to connect".to_string()
                } else {
                    "Paste an API key to connect".to_string()
                },
                free_tier: k.free_tier,
                key_url: k.key_url.clone(),
            };
            if k.configured {
                ready.push(card);
            } else if k.free_tier {
                free.push(card);
            } else {
                paid.push(card);
            }
        }

        let l = &self.local;
        if l.ollama_installed && !l.models.is_empty() {
            ready.push(ModelSource {
                id: "ollama".to_string(),
                name: "Local models".to_string(),
                kind: SourceKind::Local,
                state: SourceState::Ready,
                detail: format!(
                    "Ollama · {} model{} on this machine",
                    l.models.len(),
                    if l.models.len() == 1 { "" } else { "s" }
                ),
                free_tier: true,
                key_url: None,
            });
        } else if let Some(rec) = &l.recommended {
            local.push(ModelSource {
                id: "ollama".to_string(),
                name: "Local model".to_string(),
                kind: SourceKind::Local,
                state: SourceState::NeedsSetup,
                detail: format!("Free, private, runs here. Fits this machine: {}", rec.label),
                free_tier: true,
                key_url: None,
            });
        }

        let mut out = ready;
        out.extend(login);
        out.extend(free);
        out.extend(subs);
        out.extend(local);
        out.extend(paid);
        out.extend(missing);
        out
    }

    /// Claude Code is installed and signed in on its own, but Forge has no token of its own for
    /// it: the "reconnect" case for anyone updating from before the bridge needed one.
    pub fn claude_needs_reconnect(&self) -> bool {
        self.bridges.iter().any(|b| {
            b.id == "claude-cli"
                && b.installed
                && b.main_login
                && !b.token_stored
                && b.login != Login::SignedIn
        })
    }

    /// Cards that are usable right now.
    pub fn ready_sources(&self) -> Vec<ModelSource> {
        self.model_sources()
            .into_iter()
            .filter(|s| s.state == SourceState::Ready)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge(id: &str, installed: bool, login: Login) -> BridgeFacts {
        BridgeFacts {
            id: id.to_string(),
            name: match id {
                "claude-cli" => "Claude Code",
                "codex-cli" => "Codex",
                _ => "Antigravity",
            }
            .to_string(),
            binary: id.trim_end_matches("-cli").to_string(),
            installed,
            version: installed.then(|| "2.1.4".to_string()),
            login,
            token_stored: false,
            main_login: false,
        }
    }

    fn key(id: &str, configured: bool, free: bool) -> KeyFacts {
        KeyFacts {
            id: id.to_string(),
            name: id.to_string(),
            configured,
            from_env: false,
            free_tier: free,
            key_url: Some(format!("https://example.test/{id}")),
            featured: true,
        }
    }

    #[test]
    fn cards_lead_with_what_works_then_what_is_one_step_away() {
        let machine = Machine {
            bridges: vec![
                bridge("claude-cli", true, Login::SignedIn),
                bridge("codex-cli", true, Login::SignedOut),
                bridge("agy-cli", false, Login::Unknown),
            ],
            keys: vec![
                key("anthropic", false, false),
                key("groq", false, true),
                key("openrouter", true, true),
            ],
            ..Machine::default()
        };
        let ids: Vec<String> = machine.model_sources().into_iter().map(|s| s.id).collect();
        assert_eq!(
            ids,
            [
                "claude-cli",
                "openrouter",
                "codex-cli",
                "groq",
                "anthropic",
                "agy-cli"
            ]
        );
        let first = &machine.model_sources()[0];
        assert_eq!(first.detail, "Claude Code 2.1.4 · signed in");
        assert_eq!(machine.ready_sources().len(), 2);
    }

    #[test]
    fn a_signed_out_bridge_is_not_ready() {
        let machine = Machine {
            bridges: vec![bridge("claude-cli", true, Login::SignedOut)],
            ..Machine::default()
        };
        assert!(machine.ready_sources().is_empty());
        assert_eq!(machine.model_sources()[0].state, SourceState::NeedsLogin);
    }

    #[test]
    fn an_unknown_login_is_offered_as_installed_and_left_to_verification() {
        let machine = Machine {
            bridges: vec![bridge("claude-cli", true, Login::Unknown)],
            ..Machine::default()
        };
        let card = &machine.model_sources()[0];
        assert_eq!(card.state, SourceState::Ready);
        assert_eq!(card.detail, "Claude Code 2.1.4 · installed");
    }

    #[test]
    fn local_models_are_ready_when_pulled_and_a_recommendation_otherwise() {
        let mut machine = Machine::default();
        machine.local.ollama_installed = true;
        machine.local.models = vec!["qwen2.5-coder:14b".into()];
        assert_eq!(machine.model_sources()[0].state, SourceState::Ready);
        assert!(machine.model_sources()[0].detail.contains("1 model on"));

        let mut machine = Machine::default();
        machine.local.recommended = Some(LocalRecommendation {
            tag: "qwen2.5-coder:14b".into(),
            label: "Qwen2.5-Coder 14B".into(),
            memory_gb: 16.0,
        });
        let card = &machine.model_sources()[0];
        assert_eq!(card.state, SourceState::NeedsSetup);
        assert!(card.detail.contains("Qwen2.5-Coder 14B"));
    }

    #[test]
    fn an_unconfigured_key_provider_that_is_not_featured_gets_no_card() {
        let mut k = key("cohere", false, false);
        k.featured = false;
        let machine = Machine {
            keys: vec![k],
            ..Machine::default()
        };
        assert!(machine.model_sources().is_empty());
    }

    #[test]
    fn the_machine_round_trips_through_json_and_tolerates_missing_fields() {
        let machine = Machine {
            os: "linux".into(),
            bridges: vec![bridge("claude-cli", true, Login::SignedIn)],
            ..Machine::default()
        };
        let json = serde_json::to_string(&machine).unwrap();
        assert_eq!(serde_json::from_str::<Machine>(&json).unwrap(), machine);
        assert_eq!(
            serde_json::from_str::<Machine>("{}").unwrap(),
            Machine::default()
        );
    }
}
