//! State for the right-hand sidebar (Ctrl+B): whether it is on and the MCP server health it
//! shows. The rows are drawn by `app::render::panels::sidebar`, from the live `App`.

/// The sidebar opens only on terminals at least this wide; narrower, there is no room for it
/// beside a readable transcript.
pub const MIN_TERMINAL: u16 = 120;
/// Width of the sidebar, its rule column included.
pub const WIDTH: u16 = 34;

/// One MCP server's health dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpDot {
    pub name: String,
    /// `connected`, `reconnecting`, `slow`, `unauthorized`, `failed` or `disabled`.
    pub status: String,
}

/// What the sidebar remembers between frames.
#[derive(Debug, Clone, Default)]
pub struct SidebarState {
    pub enabled: bool,
    pub mcp: Vec<McpDot>,
}

impl SidebarState {
    /// Replace the health dots from the session's MCP status lines.
    pub fn set_mcp(&mut self, servers: &[forge_types::McpServerLine]) {
        self.mcp = servers
            .iter()
            .map(|s| McpDot {
                name: s.name.clone(),
                status: s.status.clone(),
            })
            .collect();
    }

    /// Whether the sidebar takes columns on a frame `width` wide.
    pub fn visible(&self, fullscreen: bool, width: u16) -> bool {
        self.enabled && fullscreen && width >= MIN_TERMINAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_room_and_the_full_screen_transcript() {
        let mut s = SidebarState::default();
        assert!(!s.visible(true, 200));
        s.enabled = true;
        assert!(s.visible(true, MIN_TERMINAL));
        assert!(!s.visible(true, MIN_TERMINAL - 1));
        assert!(!s.visible(false, 200));
    }

    #[test]
    fn mcp_lines_become_dots() {
        let mut s = SidebarState::default();
        s.set_mcp(&[forge_types::McpServerLine {
            name: "github".into(),
            status: "connected".into(),
            transport: "http".into(),
            tools: 3,
            resources: 0,
            prompts: 0,
            detail: None,
        }]);
        assert_eq!(s.mcp[0].name, "github");
    }
}
