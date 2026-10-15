//! The live subject: a real `Session` (the direct-API path the TUI drives) inside an isolated
//! environment: a temp workspace, temp store, temp config/data/cache dirs, no fleet publishing.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use forge_core::Session;
use forge_types::{ConfirmOutcome, Presenter, PresenterEvent, QChoice, SideEffect, NO_ANSWER};

use super::driver::{Subject, TurnOutcome};
use super::score::UsageTotals;
use crate::Mode;

const TURN_TIMEOUT: Duration = Duration::from_secs(300);

/// Swallows every event (the bench reads results from the session, not from a screen) and
/// answers confirmations itself, so a turn can never block on a human.
struct BenchPresenter;

impl Presenter for BenchPresenter {
    fn emit(&mut self, event: PresenterEvent) {
        if let PresenterEvent::Error(e) = event {
            eprintln!("  session error: {e}");
        }
    }

    fn confirm(&mut self, _tool: &str, _side_effect: SideEffect) -> ConfirmOutcome {
        ConfirmOutcome::Allow
    }

    fn ask(&mut self, _question: &str, _options: &[QChoice], _allow_other: bool) -> String {
        NO_ANSWER.to_string()
    }

    fn read_line(&mut self) -> Option<String> {
        None
    }

    fn is_attended(&self) -> bool {
        false
    }

    fn consumes_recap(&self) -> bool {
        false
    }

    fn consumes_suggestions(&self) -> bool {
        false
    }
}

/// Model price in USD per million tokens, written into the temp config so the store records
/// a cost for models the catalog has no price for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Price {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
}

/// The config the bench session runs with: defaults, minus everything that would make runs
/// differ between machines or leave traces (fleet publishing, self-MCP, code index, discovery),
/// plus the one provider entry (and window/price) the pinned model needs, taken from `real`.
pub(super) fn render_config(
    real: Option<&toml::Table>,
    model: Option<&str>,
    price: Option<Price>,
) -> toml::Table {
    let mut cfg: toml::Table = toml::from_str(
        "permission_mode = \"bypass\"\nself_mcp = false\n\
         [lattice]\nenabled = false\n\
         [remote]\npublish_local_runs = false\npublish_local_sessions = false\n\
         [mesh]\nauto_discover = false\n",
    )
    .expect("static config parses");
    let Some(model) = model else {
        return cfg;
    };
    let (namespace, bare) = model.split_once("::").unwrap_or(("", model));
    let section = |t: &toml::Table, path: &[&str]| -> Option<toml::Value> {
        let mut cur: &toml::Value = t.get(*path.first()?)?;
        for key in &path[1..] {
            cur = cur.get(key)?;
        }
        Some(cur.clone())
    };
    let from_real = real.and_then(|r| {
        section(r, &["providers", "custom"])?
            .as_array()?
            .iter()
            .find(|p| p.get("namespace").and_then(|n| n.as_str()) == Some(namespace))
            .cloned()
    });
    let provider = from_real.or_else(|| {
        (namespace == "inferhub").then(|| {
            toml::from_str::<toml::Value>(&format!(
                "namespace = \"inferhub\"\nbase_url = \"https://api.inferhub.dev/v1\"\n\
                 api_key_env = \"INFERHUB_API_KEY\"\nlabel = \"InferHub\"\nmodels = [\"{bare}\"]\n"
            ))
            .expect("static provider parses")
        })
    });
    if let Some(provider) = provider {
        let mut providers = toml::Table::new();
        providers.insert("custom".into(), toml::Value::Array(vec![provider]));
        cfg.insert("providers".into(), toml::Value::Table(providers));
    }
    let mesh = cfg
        .get_mut("mesh")
        .and_then(|m| m.as_table_mut())
        .expect("mesh table");
    if let Some(window) = real.and_then(|r| section(r, &["mesh", "context_windows", model])) {
        let mut windows = toml::Table::new();
        windows.insert(model.to_string(), window);
        mesh.insert("context_windows".into(), toml::Value::Table(windows));
    }
    let pricing = match price {
        Some(p) => {
            let mut t = toml::Table::new();
            t.insert("input_per_1k".into(), (p.input_per_mtok / 1000.0).into());
            t.insert("output_per_1k".into(), (p.output_per_mtok / 1000.0).into());
            Some(toml::Value::Table(t))
        }
        None => real.and_then(|r| section(r, &["mesh", "pricing", model])),
    };
    if let Some(pricing) = pricing {
        let mut all = toml::Table::new();
        all.insert(model.to_string(), pricing);
        mesh.insert("pricing".into(), toml::Value::Table(all));
    }
    cfg
}

const ENV_VARS: [&str; 5] = [
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "FORGE_DB",
];

/// Redirects the process into a throwaway environment and puts everything back on drop.
pub(super) struct IsolatedEnv {
    _root: tempfile::TempDir,
    saved: Vec<(&'static str, Option<OsString>)>,
    cwd: Option<PathBuf>,
}

impl IsolatedEnv {
    pub(super) fn enter(model: Option<&str>, price: Option<Price>) -> Result<Self> {
        // Read what is borrowed from the real setup before the environment moves.
        let real_dir = forge_config::config_dir();
        let real_cfg = real_dir
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("config.toml")).ok())
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok());
        let cfg = render_config(real_cfg.as_ref(), model, price);

        let root = tempfile::Builder::new()
            .prefix("forge-recall-env-")
            .tempdir()
            .context("creating the isolated environment")?;
        let cfg_dir = root.path().join("config/forge");
        std::fs::create_dir_all(&cfg_dir)?;
        std::fs::write(cfg_dir.join("config.toml"), toml::to_string(&cfg)?)?;
        // The encrypted key store is looked up under the config dir; a copy keeps the provider's
        // API key reachable without exposing the rest of the real config.
        if let Some(real_dir) = &real_dir {
            for name in ["secrets.enc", "secret.key"] {
                let _ = std::fs::copy(real_dir.join(name), cfg_dir.join(name));
            }
        }

        let saved = ENV_VARS.iter().map(|k| (*k, std::env::var_os(k))).collect();
        let at = |sub: &str| root.path().join(sub);
        std::env::set_var("XDG_CONFIG_HOME", at("config"));
        std::env::set_var("XDG_DATA_HOME", at("data"));
        std::env::set_var("XDG_STATE_HOME", at("state"));
        std::env::set_var("XDG_CACHE_HOME", at("cache"));
        std::env::set_var("FORGE_DB", at("forge.db"));
        Ok(Self {
            _root: root,
            saved,
            cwd: std::env::current_dir().ok(),
        })
    }
}

impl Drop for IsolatedEnv {
    fn drop(&mut self) {
        for (key, value) in &self.saved {
            match value {
                Some(v) => std::env::set_var(key, v),
                None => std::env::remove_var(key),
            }
        }
        if let Some(cwd) = &self.cwd {
            let _ = std::env::set_current_dir(cwd);
        }
    }
}

pub(super) struct LiveSubject {
    session: Option<Session>,
    session_id: String,
    workdir: PathBuf,
    pin: Option<String>,
    mock: bool,
}

impl LiveSubject {
    pub(super) async fn start(workdir: &Path, pin: Option<String>, mock: bool) -> Result<Self> {
        std::env::set_current_dir(workdir).context("entering the bench workspace")?;
        let session = Self::build(mock, None, pin.clone()).await?;
        Ok(Self {
            session_id: session.session_id().to_string(),
            session: Some(session),
            workdir: workdir.to_path_buf(),
            pin,
            mock,
        })
    }

    async fn build(mock: bool, resume: Option<String>, pin: Option<String>) -> Result<Session> {
        crate::cli::commands::run::build_session_with(
            Box::new(BenchPresenter),
            mock,
            Some(Mode::Bypass),
            resume,
            pin,
            true,
        )
        .await
        .context("building the bench session")
    }

    fn session(&mut self) -> &mut Session {
        self.session.as_mut().expect("a live session")
    }
}

impl Subject for LiveSubject {
    async fn turn(&mut self, prompt: &str) -> TurnOutcome {
        let session = self.session();
        let calls_before = session.session_usage_detail().tool_calls;
        let ran = tokio::time::timeout(TURN_TIMEOUT, session.run_turn(prompt)).await;
        let tool_calls = session
            .session_usage_detail()
            .tool_calls
            .saturating_sub(calls_before);
        match ran {
            Ok(Ok(outcome)) => {
                let text = if outcome.text.trim().is_empty() {
                    session
                        .last_assistant_text()
                        .unwrap_or_default()
                        .to_string()
                } else {
                    outcome.text
                };
                TurnOutcome {
                    text,
                    tool_calls,
                    error: None,
                }
            }
            Ok(Err(e)) => TurnOutcome {
                text: String::new(),
                tool_calls,
                error: Some(format!("{e}")),
            },
            Err(_) => TurnOutcome {
                text: String::new(),
                tool_calls,
                error: Some(format!("turn exceeded {}s", TURN_TIMEOUT.as_secs())),
            },
        }
    }

    async fn compact(&mut self) -> Result<(usize, usize)> {
        self.session().compact(false).await.map_err(Into::into)
    }

    async fn undo(&mut self) -> Result<()> {
        match self.session().undo()? {
            Some(_) => Ok(()),
            None => anyhow::bail!("nothing to undo"),
        }
    }

    async fn resume(&mut self) -> Result<()> {
        drop(self.session.take());
        let session =
            Self::build(self.mock, Some(self.session_id.clone()), self.pin.clone()).await?;
        self.session = Some(session);
        Ok(())
    }

    fn workdir(&self) -> &Path {
        &self.workdir
    }

    fn usage(&self) -> UsageTotals {
        let Some(session) = self.session.as_ref() else {
            return UsageTotals::default();
        };
        let d = session.session_usage_detail();
        UsageTotals {
            input_tokens: d.input_tokens,
            output_tokens: d.output_tokens,
            cached_input_tokens: d.cached_input_tokens.unwrap_or(0),
            reasoning_tokens: d.reasoning_tokens.unwrap_or(0),
            model_calls: d.model_calls,
            tool_calls: d.tool_calls,
            cost_usd: d.cost_usd,
        }
    }

    fn session_id(&self) -> Option<String> {
        Some(self.session_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn real() -> toml::Table {
        toml::from_str(
            r#"
            permission_mode = "ask"
            [[providers.custom]]
            namespace = "other"
            base_url = "http://other/v1"
            [[providers.custom]]
            namespace = "inferhub"
            base_url = "https://example.test/v1"
            api_key_env = "INFERHUB_API_KEY"
            [mesh.context_windows]
            "inferhub::flash" = 123000
            "inferhub::other" = 5
            [mesh.pricing."inferhub::flash"]
            input_per_1k = 0.5
            output_per_1k = 0.7
            [system_prompt_overrides]
            inferhub = "@/somewhere/SYSTEM.md"
            "#,
        )
        .unwrap()
    }

    fn get<'a>(t: &'a toml::Table, path: &[&str]) -> Option<&'a toml::Value> {
        let mut cur = t.get(path[0])?;
        for k in &path[1..] {
            cur = cur.get(k)?;
        }
        Some(cur)
    }

    #[test]
    fn the_config_is_isolated_and_borrows_only_what_the_model_needs() {
        let cfg = render_config(Some(&real()), Some("inferhub::flash"), None);
        assert_eq!(
            get(&cfg, &["permission_mode"]).unwrap().as_str(),
            Some("bypass")
        );
        assert_eq!(
            get(&cfg, &["remote", "publish_local_runs"])
                .unwrap()
                .as_bool(),
            Some(false)
        );
        assert_eq!(
            get(&cfg, &["lattice", "enabled"]).unwrap().as_bool(),
            Some(false)
        );
        assert_eq!(get(&cfg, &["self_mcp"]).unwrap().as_bool(), Some(false));
        let custom = get(&cfg, &["providers", "custom"])
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(custom.len(), 1, "only the provider of the pinned model");
        assert_eq!(
            custom[0]["base_url"].as_str(),
            Some("https://example.test/v1")
        );
        assert_eq!(
            get(&cfg, &["mesh", "context_windows", "inferhub::flash"])
                .unwrap()
                .as_integer(),
            Some(123000)
        );
        assert!(get(&cfg, &["mesh", "context_windows", "inferhub::other"]).is_none());
        assert_eq!(
            get(
                &cfg,
                &["mesh", "pricing", "inferhub::flash", "input_per_1k"]
            )
            .unwrap()
            .as_float(),
            Some(0.5)
        );
        assert!(
            cfg.get("system_prompt_overrides").is_none(),
            "user prompt overrides stay behind"
        );
    }

    #[test]
    fn inferhub_falls_back_to_its_public_endpoint_and_flags_override_the_price() {
        let price = Price {
            input_per_mtok: 100.0,
            output_per_mtok: 400.0,
        };
        let cfg = render_config(None, Some("inferhub::glm-5.3-flash"), Some(price));
        let custom = get(&cfg, &["providers", "custom"])
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(
            custom[0]["base_url"].as_str(),
            Some("https://api.inferhub.dev/v1")
        );
        assert_eq!(custom[0]["models"][0].as_str(), Some("glm-5.3-flash"));
        let p = get(&cfg, &["mesh", "pricing", "inferhub::glm-5.3-flash"]).unwrap();
        assert!((p["input_per_1k"].as_float().unwrap() - 0.1).abs() < 1e-12);
        assert!((p["output_per_1k"].as_float().unwrap() - 0.4).abs() < 1e-12);
    }

    #[test]
    fn an_unknown_provider_or_no_model_adds_no_provider_block() {
        assert!(render_config(None, Some("nowhere::m"), None)
            .get("providers")
            .is_none());
        assert!(render_config(Some(&real()), None, None)
            .get("providers")
            .is_none());
        let cfg = render_config(None, None, None);
        assert_eq!(
            get(&cfg, &["mesh", "auto_discover"]).unwrap().as_bool(),
            Some(false)
        );
    }
}
