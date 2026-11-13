use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::ChildStdout;

/// Upper bound on a single LSP message body. Real messages (even large `publishDiagnostics`
/// batches) are nowhere near this; it exists purely to reject a corrupt/misbehaving server's
/// bogus `Content-Length` before we `vec![0u8; len]` it, which would otherwise abort the whole
/// process on allocation failure instead of degrading to "no diagnostics".
const MAX_CONTENT_LENGTH: usize = 16 * 1024 * 1024;

pub async fn write_msg<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(std::io::Error::other)?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    writer.write_all(header.as_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await?;
    Ok(())
}

pub struct FrameReader<R> {
    reader: R,
    buf: Vec<u8>,
}

impl<R: tokio::io::AsyncRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
        }
    }

    fn take_frame(&mut self) -> std::io::Result<Option<Value>> {
        let mut content_length: Option<usize> = None;
        let mut pos = 0;
        let body_start = loop {
            let Some(rel) = self.buf[pos..].iter().position(|&b| b == b'\n') else {
                return Ok(None);
            };
            let line = String::from_utf8_lossy(&self.buf[pos..pos + rel]).into_owned();
            pos += rel + 1;
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break pos;
            }
            if let Some(val) = trimmed.strip_prefix("Content-Length: ") {
                content_length = Some(val.trim().parse().map_err(|_| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("lsp: invalid Content-Length header: {val:?}"),
                    )
                })?);
            }
        };
        let len = content_length.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "lsp: message is missing a Content-Length header",
            )
        })?;
        if len > MAX_CONTENT_LENGTH {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("lsp: Content-Length {len} exceeds the {MAX_CONTENT_LENGTH}-byte limit"),
            ));
        }
        if self.buf.len() < body_start + len {
            return Ok(None);
        }
        let parsed = serde_json::from_slice(&self.buf[body_start..body_start + len]);
        self.buf.drain(..body_start + len);
        parsed.map(Some).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("lsp: invalid JSON message body: {error}"),
            )
        })
    }

    /// Cancel-safe: bytes read before a dropped future stay in `buf` and the next call resumes.
    pub async fn read_msg(&mut self) -> std::io::Result<Option<Value>> {
        loop {
            match self.take_frame() {
                Ok(Some(msg)) => return Ok(Some(msg)),
                Ok(None) => {}
                Err(error) => {
                    self.buf.clear();
                    return Err(error);
                }
            }
            // A full Vec only grows ~64 bytes per `read_buf`; reserve so a large body arrives in
            // real chunks instead of thousands of tiny reads that each re-scan the headers.
            self.buf.reserve(8192);
            if self.reader.read_buf(&mut self.buf).await? == 0 {
                if self.buf.is_empty() {
                    return Ok(None);
                }
                let headers_done = self.buf.windows(2).any(|w| w == b"\n\n")
                    || self.buf.windows(4).any(|w| w == b"\r\n\r\n");
                let truncated = if headers_done {
                    "lsp: truncated message body"
                } else {
                    "lsp: truncated message headers"
                };
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    truncated,
                ));
            }
        }
    }
}

pub async fn read_msg(reader: &mut FrameReader<ChildStdout>) -> std::io::Result<Option<Value>> {
    reader.read_msg().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{duplex, AsyncWriteExt};

    /// Frame `bytes` through a closed in-memory stream and return what `read_msg_inner` makes of it.
    async fn parse_bytes(bytes: &[u8]) -> std::io::Result<Option<Value>> {
        let (mut writer, reader) = duplex(64 * 1024);
        writer.write_all(bytes).await.unwrap();
        drop(writer);
        FrameReader::new(reader).read_msg().await
    }

    #[tokio::test]
    async fn round_trip() {
        let (mut writer, reader) = duplex(4096);
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "test",
            "params": null
        });
        write_msg(&mut writer, &msg).await.unwrap();
        drop(writer);
        let result = FrameReader::new(reader)
            .read_msg()
            .await
            .expect("should read")
            .expect("should parse");
        assert_eq!(result["method"], "test");
        assert_eq!(result["id"], 1);
    }

    #[tokio::test]
    async fn write_msg_emits_content_length_header() {
        let mut out: Vec<u8> = Vec::new();
        let msg = serde_json::json!({"a": 1});
        write_msg(&mut out, &msg).await.unwrap();
        let body = serde_json::to_vec(&msg).unwrap();
        let expected_header = format!("Content-Length: {}\r\n\r\n", body.len());
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with(&expected_header), "header was: {text:?}");
        assert!(text.ends_with(&String::from_utf8(body).unwrap()));
    }

    #[tokio::test]
    async fn eof_yields_none() {
        assert!(parse_bytes(b"").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn headers_without_content_length_return_error() {
        // A complete header block (blank line) but no Content-Length is a protocol error.
        let error = parse_bytes(b"Content-Type: application/json\r\n\r\n")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("missing a Content-Length"));
    }

    #[tokio::test]
    async fn invalid_content_length_returns_error() {
        let error = parse_bytes(b"Content-Length: not-a-number\r\n\r\n{}")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("invalid Content-Length"));
    }

    #[tokio::test]
    async fn oversized_content_length_returns_error() {
        // A bogus/corrupt header claiming far more than MAX_CONTENT_LENGTH must be rejected
        // before allocating, not passed to `vec![0u8; len]`.
        let header = format!("Content-Length: {}\r\n\r\n", MAX_CONTENT_LENGTH + 1);
        let error = parse_bytes(header.as_bytes()).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn partial_body_returns_unexpected_eof() {
        // Declares 100 bytes but only 2 follow => preserve the read_exact failure.
        let error = parse_bytes(b"Content-Length: 100\r\n\r\n{}")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn garbage_body_returns_invalid_data() {
        // 13-byte body that isn't valid JSON.
        let error = parse_bytes(b"Content-Length: 13\r\n\r\nnot-json-here")
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("invalid JSON message body"));
    }

    #[tokio::test]
    async fn ignores_extra_headers_and_parses_body() {
        let body = br#"{"jsonrpc":"2.0","id":7}"#;
        let mut bytes = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        let v = parse_bytes(&bytes)
            .await
            .expect("should read")
            .expect("should parse");
        assert_eq!(v["id"], 7);
    }

    #[tokio::test]
    async fn reads_only_declared_body_then_stops() {
        // Only `len` bytes belong to the first message; the rest is a second frame.
        let body = br#"{"id":1}"#;
        let mut bytes = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);
        bytes.extend_from_slice(b"Content-Length: 8\r\n\r\n{\"id\":2}");
        let v = parse_bytes(&bytes)
            .await
            .expect("should read")
            .expect("should parse first");
        assert_eq!(v["id"], 1);
    }

    #[tokio::test]
    async fn handles_multibyte_utf8_body() {
        // Content-Length is a BYTE count; a multibyte char must be framed by bytes, not chars.
        let (mut writer, reader) = duplex(4096);
        let msg = serde_json::json!({"msg": "café 你好"});
        write_msg(&mut writer, &msg).await.unwrap();
        drop(writer);
        let v = FrameReader::new(reader)
            .read_msg()
            .await
            .expect("should read")
            .expect("should parse");
        assert_eq!(v["msg"], "café 你好");
    }

    #[tokio::test]
    async fn timed_out_read_resumes_mid_frame() {
        let (mut writer, reader) = duplex(4096);
        let mut reader = FrameReader::new(reader);
        let body = br#"{"id":1}"#;
        writer.write_all(b"Content-Len").await.unwrap();
        let first =
            tokio::time::timeout(std::time::Duration::from_millis(50), reader.read_msg()).await;
        assert!(first.is_err());
        let mut rest = format!("gth: {}\r\n\r\n", body.len()).into_bytes();
        rest.extend_from_slice(&body[..3]);
        writer.write_all(&rest).await.unwrap();
        let second =
            tokio::time::timeout(std::time::Duration::from_millis(50), reader.read_msg()).await;
        assert!(second.is_err());
        writer.write_all(&body[3..]).await.unwrap();
        let v = reader.read_msg().await.unwrap().unwrap();
        assert_eq!(v["id"], 1);
    }
}
