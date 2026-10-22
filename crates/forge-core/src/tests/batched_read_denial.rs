//! One denied entry in a batched `read_file` withholds only itself.

use super::*;

/// Reads a batch of files in one call, then answers.
struct BatchReader {
    calls: std::sync::atomic::AtomicUsize,
    paths: Vec<String>,
}

#[async_trait::async_trait]
impl Provider for BatchReader {
    async fn complete(
        &self,
        _model: &str,
        _messages: &[Message],
        _tools: &[ToolSpec],
        _on_event: &mut forge_provider::EventSink<'_>,
    ) -> Result<forge_provider::ModelResponse, forge_provider::ProviderError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let tool_calls = if n == 0 {
            vec![forge_types::ToolCall {
                id: forge_types::new_id(),
                name: "read_file".into(),
                args: serde_json::json!({ "paths": self.paths }),
            }]
        } else {
            Vec::new()
        };
        Ok(forge_provider::ModelResponse {
            reasoning: String::new(),
            reasoning_items: Vec::new(),
            content: if n == 0 { String::new() } else { "read".into() },
            tool_calls,
            usage: forge_types::Usage::default(),
            quotas: Vec::new(),
        })
    }
}

async fn read_batch(paths: &[&str]) -> (String, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("forge-batch-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "alpha-contents").unwrap();
    std::fs::write(dir.join(".env"), "SECRET=hunter2").unwrap();
    std::fs::write(dir.join(".env.example"), "SECRET=changeme").unwrap();
    let paths: Vec<String> = paths
        .iter()
        .map(|p| dir.join(p).to_string_lossy().into_owned())
        .collect();
    let mut config = Config {
        permission_mode: forge_types::PermissionMode::Bypass,
        ..Config::default()
    };
    config.recap.enabled = false;
    config.suggest.enabled = false;
    config.mesh.auto_memory = false;
    let mut session = Session::start(
        Arc::new(Store::open_in_memory().unwrap()),
        Arc::new(BatchReader {
            calls: std::sync::atomic::AtomicUsize::new(0),
            paths,
        }),
        Arc::new(FixedRouter {
            model: "m::x".into(),
            fallbacks: vec![],
        }),
        ToolRegistry::with_core_tools_in(&dir),
        Box::new(CapturePresenter::default()),
        config,
        dir.to_str().unwrap(),
    )
    .unwrap();
    session.run_turn("read the files").await.unwrap();
    let tool_result = session
        .transcript
        .iter()
        .find(|m| m.role == Role::Tool)
        .map(|m| m.content.clone())
        .expect("the read_file call produced a tool result");
    (tool_result, dir)
}

#[tokio::test]
async fn a_denied_entry_is_withheld_and_the_rest_of_the_batch_is_read() {
    let (result, dir) = read_batch(&["a.txt", ".env", ".env.example"]).await;
    assert!(result.contains("alpha-contents"), "{result}");
    assert!(
        result.contains("SECRET=changeme"),
        "the template is readable: {result}"
    );
    assert!(
        !result.contains("hunter2"),
        "the real .env must not be read: {result}"
    );
    assert!(
        result.contains("withheld by policy, 1 of 3 paths"),
        "the model is told what was withheld: {result}"
    );
    assert!(result.contains(".env` (built-in deny rule)"), "{result}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_batch_of_only_denied_paths_keeps_the_ordinary_refusal() {
    let (result, dir) = read_batch(&[".env"]).await;
    assert!(result.contains("permission denied by policy"), "{result}");
    assert!(
        result.contains("matched Forge's built-in deny rule"),
        "{result}"
    );
    assert!(!result.contains("hunter2"), "{result}");
    let _ = std::fs::remove_dir_all(&dir);
}
