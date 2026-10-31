//! The control toolset over HTTP, for processes that are not the daemon: a terminal `forge chat`
//! session, the `forge mcp-serve` bridge a CLI model talks to, and `forge mcp control`. They
//! forward each call to the running daemon, which executes it and writes the audit line
//! (`crate::serve_control`).

use forge_core::control::ControlHost;
use serde_json::{json, Value};

pub(crate) struct HttpControl {
    /// `(base url, token)` instead of the configured local daemon; tests point it at a scratch
    /// router.
    daemon: Option<(String, String)>,
}

impl HttpControl {
    pub(crate) fn local() -> Self {
        Self { daemon: None }
    }

    #[cfg(test)]
    pub(crate) fn at(base: String, token: String) -> Self {
        Self {
            daemon: Some((base, token)),
        }
    }

    fn url(&self, path: &str) -> Result<String, String> {
        let (base, token) = match &self.daemon {
            Some(pair) => pair.clone(),
            None => (
                crate::attach::resolve_base_url(None),
                crate::attach::resolve_token(None).map_err(|e| e.to_string())?,
            ),
        };
        Ok(format!("{base}/{token}{path}"))
    }

    /// Run `tool` on the daemon. `Ok` text is the result, `Err` text is the refusal or failure.
    pub(crate) async fn run(&self, me: &str, tool: &str, args: &Value) -> Result<String, String> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .map_err(|e| e.to_string())?;
        let response = http
            .post(self.url("/api/control/call")?)
            .json(&json!({"tool": tool, "args": args, "me": me}))
            .send()
            .await
            .map_err(|e| {
                format!("could not reach the forge serve daemon - is it running? [{e}]")
            })?;
        if !response.status().is_success() {
            return Err(format!("daemon answered {}", response.status()));
        }
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        let text = body["text"].as_str().unwrap_or_default().to_string();
        if body["ok"].as_bool().unwrap_or(false) {
            Ok(text)
        } else {
            Err(text)
        }
    }
}

#[async_trait::async_trait]
impl ControlHost for HttpControl {
    fn disabled(&self) -> Vec<String> {
        crate::control_settings::disabled()
    }

    async fn call(&self, me: &str, tool: &str, args: &Value) -> Result<String, String> {
        self.run(me, tool, args).await
    }
}
