//! The live proxy: start mitmdump, query what it caught, change what it does, replay what it saw.

use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::{flows, rules::InterceptRules, Filter, Flow, ADDON, DEFAULT_PORT};

/// A running mitmdump and the two files it talks through.
pub struct Proxy {
    child: tokio::process::Child,
    pub port: u16,
    pub capture: PathBuf,
    pub rules_path: PathBuf,
    pub rules: InterceptRules,
    http: reqwest::Client,
}

/// What `status` reports.
#[derive(Debug, Clone)]
pub struct ProxyStatus {
    pub port: u16,
    pub listening_on: String,
    pub captured: usize,
    pub rules: String,
    pub capture_path: PathBuf,
}

impl Proxy {
    /// Start mitmdump on `port`, writing captures under `dir`.
    ///
    /// Binds all interfaces, not loopback: the whole point is a *phone* reaching it, and a
    /// loopback-only proxy is invisible to every device except this machine. That is a deliberate
    /// exposure — anyone on the LAN can use it as an open proxy while it runs — which is why it is
    /// started explicitly per task rather than left running.
    pub async fn start(port: Option<u16>, dir: &std::path::Path) -> Result<Self> {
        let mitmdump = crate::find_mitmdump()?;
        let port = port.unwrap_or(DEFAULT_PORT);
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;

        let addon = dir.join("forge_addon.py");
        std::fs::write(&addon, ADDON).context("writing the capture addon")?;
        let capture = dir.join("capture.jsonl");
        let rules_path = dir.join("rules.json");
        let rules = InterceptRules::default();
        rules.write(&rules_path)?;
        // Start from an empty capture so a listing reflects THIS run. The previous run's flows
        // would otherwise show up as if the app had just made them.
        let _ = std::fs::remove_file(&capture);

        let child = tokio::process::Command::new(&mitmdump)
            .arg("--listen-host")
            .arg("0.0.0.0")
            .arg("-p")
            .arg(port.to_string())
            .arg("-s")
            .arg(&addon)
            // mitmdump's own stdout is noise once the addon is writing structured flows, and a
            // full pipe would block the proxy mid-capture.
            .arg("-q")
            .env("FORGE_PROXY_CAPTURE", &capture)
            .env("FORGE_PROXY_RULES", &rules_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting {}", mitmdump.display()))?;

        let mut proxy = Self {
            child,
            port,
            capture,
            rules_path,
            rules,
            http: reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .build()
                .unwrap_or_default(),
        };
        proxy.wait_until_listening(port).await?;
        Ok(proxy)
    }

    /// Wait for the port to accept a connection, so `start` never returns a proxy the device
    /// cannot reach yet — a phone configured against a not-yet-listening port fails once and
    /// often will not retry until the user toggles Wi-Fi.
    async fn wait_until_listening(&mut self, port: u16) -> Result<()> {
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                return Ok(());
            }
            if let Ok(Some(status)) = self.child.try_wait() {
                let stderr = match self.child.stderr.take() {
                    Some(mut pipe) => {
                        use tokio::io::AsyncReadExt;
                        let mut buf = String::new();
                        let _ = pipe.read_to_string(&mut buf).await;
                        buf
                    }
                    None => String::new(),
                };
                anyhow::bail!(
                    "mitmdump exited immediately ({status}). {}",
                    if stderr.trim().is_empty() {
                        "No output — check that `mitmdump --version` runs.".to_string()
                    } else {
                        stderr.trim().to_string()
                    }
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        anyhow::bail!("mitmdump did not start listening on port {port} within 10s")
    }

    pub fn status(&self) -> ProxyStatus {
        let host = crate::lan_ip().unwrap_or_else(|| "127.0.0.1".to_string());
        ProxyStatus {
            port: self.port,
            listening_on: format!("{host}:{}", self.port),
            captured: flows::read_capture(&self.capture)
                .map(|f| f.len())
                .unwrap_or(0),
            rules: self.rules.describe(),
            capture_path: self.capture.clone(),
        }
    }

    pub fn flows(&self, filter: &Filter, limit: usize) -> Result<Vec<Flow>> {
        let mut all = flows::read_capture(&self.capture)?;
        all.retain(|flow| filter.matches(flow));
        // Newest last: a reversing session reads a capture like a transcript, and the flow you
        // just triggered on the phone is the one you want at the bottom of the list.
        if all.len() > limit {
            all.drain(..all.len() - limit);
        }
        Ok(all)
    }

    /// One flow whole, by id or unique id prefix.
    pub fn flow(&self, id: &str) -> Result<Flow> {
        let all = flows::read_capture(&self.capture)?;
        let matches: Vec<&Flow> = all.iter().filter(|f| f.id.starts_with(id)).collect();
        match matches.len() {
            0 => anyhow::bail!("no captured flow with id starting {id}"),
            1 => Ok(matches[0].clone()),
            n => anyhow::bail!("{n} flows start with {id} — use a longer id prefix"),
        }
    }

    pub fn set_rules(&mut self, rules: InterceptRules) -> Result<()> {
        rules.write(&self.rules_path)?;
        self.rules = rules;
        Ok(())
    }

    /// Re-issue a captured request from THIS machine, optionally with changes.
    ///
    /// The point is to turn a phone's one-shot request into something iterable: change a header,
    /// send it again, see what the server says — without touching the device, and without
    /// rebuilding the request by hand from a screenshot of the headers.
    pub async fn replay(
        &self,
        id: &str,
        method: Option<&str>,
        headers: Option<&std::collections::BTreeMap<String, String>>,
        body: Option<&str>,
    ) -> Result<String> {
        let flow = self.flow(id)?;
        let method = method.unwrap_or(&flow.method).to_uppercase();
        let method = reqwest::Method::from_bytes(method.as_bytes())
            .with_context(|| format!("not an HTTP method: {method}"))?;

        let mut request = self.http.request(method, &flow.url);
        // The captured headers first, then the overrides — so a replay reproduces the original
        // request by default and only differs where you asked it to.
        for (name, value) in &flow.request_headers {
            // Hop-by-hop and length headers belong to the ORIGINAL connection; re-sending them
            // makes the replay fail in ways that look like a server rejection.
            let skip = matches!(
                name.to_ascii_lowercase().as_str(),
                "host" | "content-length" | "connection" | "transfer-encoding"
            );
            if !skip {
                request = request.header(name, value);
            }
        }
        if let Some(extra) = headers {
            for (name, value) in extra {
                request = request.header(name, value);
            }
        }
        let payload = body
            .map(str::to_string)
            .unwrap_or(flow.request_body.clone());
        if !payload.is_empty() {
            request = request.body(payload);
        }

        let response = request.send().await.context("replaying the request")?;
        let status = response.status();
        let headers: Vec<String> = response
            .headers()
            .iter()
            .map(|(k, v)| format!("{k}: {}", v.to_str().unwrap_or("<binary>")))
            .collect();
        let text = response.text().await.unwrap_or_default();
        Ok(format!(
            "{} {}\n{}\n\n{}",
            status.as_u16(),
            status.canonical_reason().unwrap_or(""),
            headers.join("\n"),
            crate::truncate(&text, crate::MAX_BODY_CHARS)
        ))
    }

    /// Drop every captured flow, keeping the proxy up. Used to get a clean slate before the one
    /// interaction you actually care about.
    pub fn clear(&self) -> Result<()> {
        std::fs::write(&self.capture, "")?;
        Ok(())
    }

    /// Export the capture as HAR 1.2 so it opens in any browser devtools or Charles/Proxyman.
    pub fn har(&self, path: &std::path::Path) -> Result<usize> {
        let all = flows::read_capture(&self.capture)?;
        let entries: Vec<serde_json::Value> = all
            .iter()
            .map(|flow| {
                serde_json::json!({
                    "startedDateTime": iso8601(flow.at),
                    "time": 0,
                    "request": {
                        "method": flow.method,
                        "url": flow.url,
                        "httpVersion": "HTTP/1.1",
                        "cookies": [],
                        "headers": header_pairs(&flow.request_headers),
                        "queryString": [],
                        "headersSize": -1,
                        "bodySize": flow.request_body_bytes,
                        "postData": {
                            "mimeType": flow.request_headers.get("content-type")
                                .cloned().unwrap_or_default(),
                            "text": flow.request_body,
                        },
                    },
                    "response": {
                        "status": flow.status.unwrap_or(0),
                        "statusText": "",
                        "httpVersion": "HTTP/1.1",
                        "cookies": [],
                        "headers": header_pairs(&flow.response_headers),
                        "content": {
                            "size": flow.response_body_bytes,
                            "mimeType": flow.response_headers.get("content-type")
                                .cloned().unwrap_or_default(),
                            "text": flow.response_body,
                        },
                        "redirectURL": "",
                        "headersSize": -1,
                        "bodySize": flow.response_body_bytes,
                    },
                    "cache": {},
                    "timings": {"send": 0, "wait": 0, "receive": 0},
                })
            })
            .collect();
        let har = serde_json::json!({
            "log": {
                "version": "1.2",
                "creator": {"name": "forge", "version": env!("CARGO_PKG_VERSION")},
                "entries": entries,
            }
        });
        std::fs::write(path, serde_json::to_string_pretty(&har)?)?;
        Ok(all.len())
    }
}

fn header_pairs(map: &std::collections::BTreeMap<String, String>) -> Vec<serde_json::Value> {
    map.iter()
        .map(|(name, value)| serde_json::json!({"name": name, "value": value}))
        .collect()
}

fn iso8601(epoch_secs: f64) -> String {
    let secs = epoch_secs as i64;
    // A HAR consumer only needs a well-formed timestamp; deriving one without a date dependency
    // keeps this crate's dependency list to what the proxy actually needs.
    format!("1970-01-01T00:00:00.000Z+{secs}")
}

impl Drop for Proxy {
    fn drop(&mut self) {
        // `kill_on_drop` handles the child; the capture stays on disk deliberately, so a HAR can
        // still be exported from a session whose proxy has already been stopped.
        let _ = self.child.start_kill();
    }
}
