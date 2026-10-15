//! Image delivery for the CLI bridges. The bridges flatten the transcript to text, so an image
//! attached to a user turn used to vanish silently; here it rides the CLI's own image channel
//! instead — claude's stream-json content blocks, codex's `--image <file>`.
//!
//! The same one-image-turn budget the API providers apply ([`last_image_turn`]) holds here: only
//! the latest image turn goes out, and only when it is part of what this invocation sends.

use base64::Engine;
use forge_types::{ImageAttachment, Message};
use serde_json::{json, Value};

use crate::system_placement::last_image_turn;

/// Images this invocation must attach: the latest image turn of `messages`, if it lies at or after
/// `from` (the first message the CLI has not already seen — 0 for a fresh session).
pub(super) fn turn_images(messages: &[Message], from: usize) -> Vec<&ImageAttachment> {
    match last_image_turn(messages) {
        Some(i) if i >= from => messages[i].images.iter().collect(),
        _ => Vec::new(),
    }
}

/// One claude stream-json user line. Text only keeps the plain-string envelope; with images the
/// content becomes blocks (images first, as Anthropic recommends).
pub(super) fn claude_user_line(payload: &str, images: &[&ImageAttachment]) -> String {
    let content = if images.is_empty() {
        Value::String(payload.to_string())
    } else {
        let mut blocks: Vec<Value> = images
            .iter()
            .map(|img| {
                json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": img.media_type,
                        "data": img.data_base64,
                    },
                })
            })
            .collect();
        blocks.push(json!({"type": "text", "text": payload}));
        Value::Array(blocks)
    };
    json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
}

/// Decoded images on disk for one `codex exec` invocation; the directory goes away with the guard.
pub(super) struct CodexImageFiles {
    dir: std::path::PathBuf,
    paths: Vec<std::path::PathBuf>,
}

impl CodexImageFiles {
    pub(super) fn write(images: &[&ImageAttachment]) -> std::io::Result<Self> {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("forge-codex-img-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let mut files = Self {
            dir,
            paths: Vec::new(),
        };
        for (i, img) in images.iter().enumerate() {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(img.data_base64.as_bytes())
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let path = files
                .dir
                .join(format!("image-{i}.{}", extension_for(&img.media_type)));
            std::fs::write(&path, bytes)?;
            files.paths.push(path);
        }
        Ok(files)
    }

    /// `codex exec --image` is variadic, so it must be the last flag with nothing positional after
    /// it; the prompt travels on stdin.
    pub(super) fn args(&self) -> Vec<String> {
        codex_image_args(&self.paths)
    }
}

impl Drop for CodexImageFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(super) fn codex_image_args(paths: &[std::path::PathBuf]) -> Vec<String> {
    let mut args = Vec::new();
    for path in paths {
        args.push("--image".to_string());
        args.push(path.display().to_string());
    }
    args
}

fn extension_for(media_type: &str) -> &'static str {
    match media_type {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => "png",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(tag: &str) -> ImageAttachment {
        ImageAttachment {
            media_type: "image/png".into(),
            data_base64: base64::engine::general_purpose::STANDARD.encode(tag),
        }
    }

    fn user_with(images: Vec<ImageAttachment>) -> Message {
        let mut m = Message::user("look");
        m.images = images;
        m
    }

    #[test]
    fn text_only_turn_keeps_the_plain_string_envelope() {
        let v: Value = serde_json::from_str(&claude_user_line("hi", &[])).unwrap();
        assert_eq!(v["message"]["content"], "hi");
    }

    #[test]
    fn image_turn_becomes_claude_content_blocks() {
        let a = img("one");
        let v: Value = serde_json::from_str(&claude_user_line("what is this", &[&a])).unwrap();
        let blocks = v["message"]["content"].as_array().unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["type"], "base64");
        assert_eq!(blocks[0]["source"]["media_type"], "image/png");
        assert_eq!(blocks[0]["source"]["data"], a.data_base64);
        assert_eq!(blocks[1]["text"], "what is this");
    }

    #[test]
    fn only_the_latest_image_turn_is_attached_and_only_if_unseen() {
        let msgs = vec![
            user_with(vec![img("old")]),
            Message::user("then text"),
            user_with(vec![img("new")]),
            Message::user("follow-up"),
        ];
        assert_eq!(turn_images(&msgs, 0).len(), 1);
        assert_eq!(turn_images(&msgs, 0)[0].data_base64, img("new").data_base64);
        assert_eq!(turn_images(&msgs, 2).len(), 1);
        assert!(turn_images(&msgs, 3).is_empty(), "already in the session");
    }

    #[test]
    fn codex_gets_one_image_flag_per_file_decoded_to_disk() {
        let a = img("pixels");
        let files = CodexImageFiles::write(&[&a]).unwrap();
        let args = files.args();
        assert_eq!(args[0], "--image");
        assert!(args[1].ends_with("image-0.png"));
        assert_eq!(std::fs::read(&args[1]).unwrap(), b"pixels");
        let dir = files.dir.clone();
        drop(files);
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    fn logging_cli(stdout: &str) -> (String, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "forge-img-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (args, stdin) = (dir.join("args"), dir.join("stdin"));
        let _ = std::fs::remove_file(&args);
        let _ = std::fs::remove_file(&stdin);
        let script = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\ncat <<'EOF'\n{stdout}\nEOF\n",
            args.display(),
            stdin.display()
        );
        let bin = super::super::tests::install_fake_cli("forge-img-cli", &script);
        (bin, args, stdin)
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_bridge_delivers_the_image_as_a_stream_json_block() {
        let (bin, args, stdin) = logging_cli(
            r#"{"type":"result","is_error":false,"result":"green and red","usage":{"input_tokens":1,"output_tokens":1}}"#,
        );
        let provider = crate::cli_provider::CliProvider::claude_code()
            .with_persistent(false)
            .with_binary(bin);
        let mut on_event = |_: crate::StreamEvent| {};
        use crate::Provider;
        provider
            .complete(
                "claude-cli::haiku",
                &[user_with(vec![img("PIXELS")])],
                &[],
                &mut on_event,
            )
            .await
            .unwrap();
        let args = std::fs::read_to_string(args).unwrap();
        assert!(args.contains("--input-format\nstream-json"), "{args}");
        let line: Value =
            serde_json::from_str(std::fs::read_to_string(stdin).unwrap().trim()).unwrap();
        let blocks = line["message"]["content"]
            .as_array()
            .expect("content blocks");
        assert_eq!(blocks[0]["type"], "image");
        assert_eq!(blocks[0]["source"]["data"], img("PIXELS").data_base64);
        assert!(blocks[1]["text"].as_str().unwrap().contains("look"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn claude_bridge_keeps_plain_stdin_when_there_is_no_image() {
        let (bin, args, stdin) = logging_cli(
            r#"{"type":"result","is_error":false,"result":"ok","usage":{"input_tokens":1,"output_tokens":1}}"#,
        );
        let provider = crate::cli_provider::CliProvider::claude_code()
            .with_persistent(false)
            .with_binary(bin);
        let mut on_event = |_: crate::StreamEvent| {};
        use crate::Provider;
        provider
            .complete(
                "claude-cli::haiku",
                &[Message::user("hi")],
                &[],
                &mut on_event,
            )
            .await
            .unwrap();
        assert!(!std::fs::read_to_string(args)
            .unwrap()
            .contains("--input-format"));
        assert!(std::fs::read_to_string(stdin).unwrap().contains("User: hi"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn codex_bridge_passes_the_image_as_an_image_file_flag() {
        let (bin, args, _stdin) = logging_cli(
            r#"{"type":"item.completed","item":{"id":"i","type":"agent_message","text":"ok"}}
{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}"#,
        );
        let provider = crate::cli_provider::CliProvider::codex().with_binary(bin);
        let mut on_event = |_: crate::StreamEvent| {};
        use crate::Provider;
        provider
            .complete(
                "codex-cli::gpt-5.6-luna",
                &[user_with(vec![img("PIXELS")])],
                &[],
                &mut on_event,
            )
            .await
            .unwrap();
        let args = std::fs::read_to_string(args).unwrap();
        let lines: Vec<&str> = args.lines().collect();
        let at = lines
            .iter()
            .position(|a| *a == "--image")
            .expect("--image flag");
        assert!(lines[at + 1].ends_with("image-0.png"), "{lines:?}");
        assert_eq!(at + 2, lines.len(), "--image is last: it is variadic");
        assert!(
            !std::path::Path::new(lines[at + 1]).exists(),
            "temp image removed after the turn"
        );
    }
}
