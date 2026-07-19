//! Live CLI-bridge tests against the real `claude` / `codex` binaries. Ignored by default —
//! they require the official CLI installed AND logged in (a subscription), and they consume
//! subscription quota. Run explicitly:
//!
//!   FORGE_CLI_BRIDGE_TESTS=1 cargo test -p forge-agent-provider --test cli_bridge_live -- --ignored
//!
//! These verify the end-to-end spawn → stream-json/JSONL parse → ModelResponse path against the
//! actual CLIs (the unit tests cover the parsers with captured fixtures + a fake binary).

use std::time::Duration;

use forge_provider::{CliProvider, Provider, StreamEvent};
use forge_types::Message;

fn enabled() -> bool {
    std::env::var("FORGE_CLI_BRIDGE_TESTS").is_ok()
}

fn forge_binary() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("FORGE_E2E_FORGE_BIN") {
        return path.into();
    }
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = if cfg!(windows) { "forge.exe" } else { "forge" };
    for profile in ["release", "debug"] {
        let candidate = workspace.join("target").join(profile).join(executable);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!("build Forge first, or set FORGE_E2E_FORGE_BIN to a Forge executable");
}

#[tokio::test]
#[ignore = "requires an authenticated `claude` CLI; run with FORGE_CLI_BRIDGE_TESTS=1 -- --ignored"]
async fn claude_cli_round_trips_text() {
    if !enabled() {
        return;
    }
    let provider = CliProvider::claude_code()
        .with_forge_binary(forge_binary().to_string_lossy())
        .with_timeout(Duration::from_secs(120));
    let mut streamed = String::new();
    let mut on_text = |ev: StreamEvent| {
        if let StreamEvent::Text(t) = ev {
            streamed.push_str(&t)
        }
    };
    let res = provider
        .complete(
            "claude-cli::",
            &[Message::user("Reply with exactly: pong")],
            &[],
            &mut on_text,
        )
        .await
        .expect("claude CLI bridge should return text");

    assert!(
        res.content.to_lowercase().contains("pong"),
        "got: {:?}",
        res.content
    );
    assert!(res.usage.input_tokens > 0, "usage should be captured");
    assert_eq!(res.usage.cost_usd, 0.0, "subscription-billed: $0 to Forge");
    assert!(res.tool_calls.is_empty(), "v1 bridge is text-only");
}

#[tokio::test]
#[ignore = "requires an authenticated `codex` CLI; run with FORGE_CLI_BRIDGE_TESTS=1 -- --ignored"]
async fn codex_cli_round_trips_text() {
    if !enabled() {
        return;
    }
    let provider = CliProvider::codex()
        .with_forge_binary(forge_binary().to_string_lossy())
        .with_timeout(Duration::from_secs(120));
    let mut streamed = String::new();
    let mut on_text = |ev: StreamEvent| {
        if let StreamEvent::Text(t) = ev {
            streamed.push_str(&t)
        }
    };
    let res = provider
        .complete(
            "codex-cli::",
            &[Message::user("Reply with exactly: pong")],
            &[],
            &mut on_text,
        )
        .await
        .expect("codex CLI bridge should return text");

    assert!(
        res.content.to_lowercase().contains("pong"),
        "got: {:?}",
        res.content
    );
    assert_eq!(res.usage.cost_usd, 0.0, "subscription-billed: $0 to Forge");
    assert!(res.tool_calls.is_empty(), "v1 bridge is text-only");
}
