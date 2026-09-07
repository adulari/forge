//! Conversions from rmcp result/content types into Forge's transport-neutral [`McpCallOutcome`]
//! and [`McpContentBlock`], plus the text shaping (`one_line`, `truncate`) the manager applies to
//! what it hands the model.

use rmcp::model::{CallToolResult, ResourceContents};

use crate::{render_blocks, McpCallOutcome, McpContentBlock, MAX_RESULT_CHARS};

pub(crate) fn tool_result_to_outcome(result: CallToolResult) -> McpCallOutcome {
    // Preserve EVERY content block (text, image, audio, embedded resource) instead of keeping only
    // text and dropping the rest. Non-text blocks keep their data + mime type in `blocks`; `text`
    // renders a typed marker for them so text-only consumers still see what came back.
    let blocks: Vec<McpContentBlock> = result.content.iter().map(content_to_block).collect();
    // An MCP `isError` payload is a tool error, not a successful result.
    if result.is_error == Some(true) {
        let mut out = McpCallOutcome::err(render_blocks(&blocks));
        out.blocks = blocks;
        out
    } else {
        McpCallOutcome::ok_blocks(blocks)
    }
}

/// Map an rmcp tool/prompt content block into Forge's structured [`McpContentBlock`], keeping the
/// raw data + mime type for non-text blocks rather than collapsing them to a placeholder string.
pub(crate) fn content_to_block(c: &rmcp::model::ContentBlock) -> McpContentBlock {
    use rmcp::model::ContentBlock;
    match c {
        ContentBlock::Text(t) => McpContentBlock::Text(t.text.clone()),
        ContentBlock::Image(i) => McpContentBlock::Image {
            data: i.data.clone(),
            mime_type: i.mime_type.clone(),
        },
        ContentBlock::Audio(a) => McpContentBlock::Audio {
            data: a.data.clone(),
            mime_type: a.mime_type.clone(),
        },
        ContentBlock::Resource(r) => resource_contents_to_block(&r.resource),
        ContentBlock::ResourceLink(l) => McpContentBlock::Resource {
            uri: l.uri.clone(),
            mime_type: l.mime_type.clone(),
            text: None,
            blob: None,
        },
        // An rmcp content variant this match doesn't cover yet — surface a marker instead of
        // silently rendering an empty string, so the model knows something was omitted.
        _ => McpContentBlock::Text("[unknown content: unsupported block type]".to_string()),
    }
}

/// Map an embedded `ResourceContents` (text or binary blob) into a structured block, preserving the
/// base64 blob + mime type for binary resources rather than dropping to a placeholder.
pub(crate) fn resource_contents_to_block(c: &ResourceContents) -> McpContentBlock {
    match c {
        ResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            ..
        } => McpContentBlock::Resource {
            uri: uri.clone(),
            mime_type: mime_type.clone(),
            text: Some(text.clone()),
            blob: None,
        },
        ResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            ..
        } => McpContentBlock::Resource {
            uri: uri.clone(),
            mime_type: mime_type.clone(),
            text: None,
            blob: Some(blob.clone()),
        },
        // Same rationale as `content_to_block`'s fallback: don't drop unhandled variants silently.
        _ => McpContentBlock::Text("[unknown resource content: unsupported variant]".to_string()),
    }
}

pub(crate) fn prompt_message_to_block(m: &rmcp::model::PromptMessage) -> McpContentBlock {
    content_to_block(&m.content)
}

pub(crate) fn one_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 120 {
        format!("{}…", line.chars().take(119).collect::<String>())
    } else {
        line.to_string()
    }
}

pub(crate) fn truncate(s: &str) -> String {
    if s.chars().count() <= MAX_RESULT_CHARS {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX_RESULT_CHARS).collect();
    format!(
        "{head}\n…[truncated {} chars]",
        s.chars().count() - MAX_RESULT_CHARS
    )
}
