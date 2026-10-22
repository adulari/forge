//! `type: "http"` hooks (Claude-Code parity): the event JSON is POSTed to `url`, and the response
//! body is read with the same rules as a command hook's stdout. Failures follow Claude Code's
//! "non-blocking error" contract: a non-2xx status, a connection failure, a timeout or a 2xx body
//! that is not a JSON object is reported and never blocks the action. To block, the endpoint
//! answers 2xx with a decision (`{"decision":"block","reason":…}`, `hookSpecificOutput`, …).

use std::time::Duration;

use forge_config::HookConfig;

use crate::hooks_exec::HookRun;

pub(crate) async fn run_http(h: &HookConfig, payload: &str) -> HookRun {
    let url = h
        .url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .ok_or("http hook has no url")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(h.timeout_secs))
        .build()
        .map_err(|e| format!("http hook client: {e}"))?;
    let mut request = client
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string());
    for (name, value) in &h.headers {
        request = request.header(name.as_str(), expand_env(value, &h.allowed_env_vars));
    }
    let response = request.send().await.map_err(|e| {
        if e.is_timeout() {
            format!("timed out after {}s", h.timeout_secs)
        } else {
            format!("http hook {url} failed: {e}")
        }
    })?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("http hook {url}: reading the response failed: {e}"))?;
    interpret(status.as_u16(), &body)
}

/// A response as a command hook result: 2xx + empty body is a silent success, 2xx + a JSON object
/// is that hook's output, anything else is a non-blocking error.
fn interpret(status: u16, body: &str) -> HookRun {
    if !(200..300).contains(&status) {
        return Err(format!("http hook returned HTTP {status}"));
    }
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok((0, String::new(), String::new()));
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(_)) => Ok((0, trimmed.to_string(), String::new())),
        _ => Err("http hook answered with a body that is not a JSON object (ignored)".to_string()),
    }
}

/// Expand `$NAME` / `${NAME}` in a header value. Only names in `allowed` are read from the
/// environment; every other reference becomes empty, so a header cannot leak a variable the
/// configuration did not name.
pub(crate) fn expand_env(value: &str, allowed: &[String]) -> String {
    let lookup = |name: &str| {
        if allowed.iter().any(|a| a == name) {
            std::env::var(name).unwrap_or_default()
        } else {
            String::new()
        }
    };
    let chars: Vec<char> = value.chars().collect();
    let mut out = String::with_capacity(value.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if chars.get(i + 1) == Some(&'{') {
            if let Some(close) = chars[i + 2..].iter().position(|c| *c == '}') {
                let name: String = chars[i + 2..i + 2 + close].iter().collect();
                out.push_str(&lookup(&name));
                i += close + 3;
                continue;
            }
        }
        let starts_name = chars
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '_');
        let name: String = chars[i + 1..]
            .iter()
            .take_while(|c| starts_name && (c.is_ascii_alphanumeric() || **c == '_'))
            .collect();
        if name.is_empty() {
            out.push('$');
            i += 1;
        } else {
            out.push_str(&lookup(&name));
            i += 1 + name.len();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_config::{HookEvent, HookKind};
    use std::io::{Read, Write};

    #[test]
    fn only_allowed_variables_expand_in_headers() {
        std::env::set_var("FORGE_HTTP_HOOK_TOKEN", "s3cret");
        std::env::set_var("FORGE_HTTP_HOOK_OTHER", "leak");
        let allowed = vec!["FORGE_HTTP_HOOK_TOKEN".to_string()];
        assert_eq!(
            expand_env("Bearer $FORGE_HTTP_HOOK_TOKEN", &allowed),
            "Bearer s3cret"
        );
        assert_eq!(
            expand_env(
                "${FORGE_HTTP_HOOK_TOKEN}-${FORGE_HTTP_HOOK_OTHER}-$5",
                &allowed
            ),
            "s3cret--$5"
        );
        assert_eq!(expand_env("cost: $", &allowed), "cost: $");
    }

    #[test]
    fn responses_map_to_command_hook_results() {
        assert_eq!(interpret(200, ""), Ok((0, String::new(), String::new())));
        assert_eq!(
            interpret(200, r#"{"decision":"block","reason":"no"}"#)
                .unwrap()
                .1,
            r#"{"decision":"block","reason":"no"}"#
        );
        assert!(interpret(200, "plain text").is_err());
        assert!(interpret(500, r#"{"decision":"block"}"#).is_err());
    }

    /// One-request HTTP server on a random port: returns the request it read via the handle.
    fn serve_once(response: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = stream.read(&mut buf).unwrap_or(0);
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                        })
                        .unwrap_or(0);
                    if seen.len() >= split + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let reply = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{response}",
                response.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&seen).to_string()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn an_http_hook_posts_the_event_and_reads_the_decision() {
        let (url, server) = serve_once(r#"{"decision":"block","reason":"policy says no"}"#);
        std::env::set_var("FORGE_HTTP_HOOK_KEY", "k-123");
        let hook = HookConfig {
            kind: HookKind::Http,
            url: Some(url),
            headers: [("X-Key".to_string(), "$FORGE_HTTP_HOOK_KEY".to_string())].into(),
            allowed_env_vars: vec!["FORGE_HTTP_HOOK_KEY".into()],
            cc_compat: true,
            ..HookConfig::new(HookEvent::PreToolUse, "")
        };
        let out = crate::hooks::run_hooks(
            std::slice::from_ref(&hook),
            HookEvent::PreToolUse,
            "shell",
            r#"{"tool":"shell","args":{"command":"rm -rf build"}}"#,
        )
        .await;
        assert_eq!(out.blocked.as_deref(), Some("policy says no"));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /hook"), "{request}");
        assert!(
            request.to_ascii_lowercase().contains("x-key: k-123"),
            "{request}"
        );
        assert!(request.contains("\"tool_name\":\"Bash\""), "{request}");
        assert!(
            request.contains("\"hook_event_name\":\"PreToolUse\""),
            "{request}"
        );
    }

    #[tokio::test]
    async fn an_unreachable_http_hook_is_a_note_not_a_block() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/hook", closed.local_addr().unwrap());
        drop(closed);
        let hook = HookConfig {
            kind: HookKind::Http,
            url: Some(url),
            timeout_secs: 5,
            ..HookConfig::new(HookEvent::PreToolUse, "")
        };
        let out = crate::hooks::run_hooks(&[hook], HookEvent::PreToolUse, "shell", "{}").await;
        assert!(out.blocked.is_none());
        assert!(
            out.notes.iter().any(|n| n.contains("hook error")),
            "{:?}",
            out.notes
        );
    }
}
