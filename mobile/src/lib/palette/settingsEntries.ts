// Settings destinations the palette can jump to. A full settings search (every row, not just
// every page) is a later addition that plugs in as another source for the Settings category.
export type SettingsHref =
  | "/settings"
  | "/appearance"
  | "/keybindings"
  | "/diagnostics"
  | "/legal"
  | "/anywhere"
  | "/anywhere/sync"
  | "/usage"
  | "/providers"
  | "/models"
  | "/plans"
  | "/mcp"
  | "/configuration"
  | "/skills"
  | "/import"
  | "/hooks"
  | "/session-tree"
  | "/workflows"
  | "/prs"
  | "/notes"
  | "/remote-access";

export interface SettingsEntry {
  href: SettingsHref;
  title: string;
  subtitle: string;
  keywords: string;
}

export const SETTINGS_ENTRIES: readonly SettingsEntry[] = [
  { href: "/settings", title: "General", subtitle: "Servers and preferences", keywords: "settings preferences servers connect pair host" },
  { href: "/appearance", title: "Appearance", subtitle: "Theme and display", keywords: "theme dark light density font accent colors" },
  { href: "/keybindings", title: "Keyboard shortcuts", subtitle: "Rebind any shortcut", keywords: "keybindings hotkeys keys rebind chord" },
  { href: "/diagnostics", title: "Diagnostics & updates", subtitle: "Daemon health and app updates", keywords: "doctor health update version logs" },
  { href: "/legal", title: "Legal & support", subtitle: "Licenses, privacy, help", keywords: "privacy terms licence license support help" },
  { href: "/anywhere", title: "Forge Anywhere", subtitle: "Remote hosts and devices", keywords: "anywhere relay remote hosts devices pair account" },
  { href: "/anywhere/sync", title: "Sync across machines", subtitle: "Keys, skills and settings on every machine", keywords: "sync account anywhere machines keys skills settings mcp instructions encrypted devices" },
  { href: "/usage", title: "Usage", subtitle: "Spend and subscription limits", keywords: "usage quota limits spend cost tokens" },
  { href: "/providers", title: "Providers & accounts", subtitle: "Model providers and logins", keywords: "providers accounts login api key anthropic openai" },
  { href: "/models", title: "Models & mesh", subtitle: "Routing and model health", keywords: "models mesh routing tiers health failover" },
  { href: "/plans", title: "Plans", subtitle: "Saved plans", keywords: "plans approval steps" },
  { href: "/mcp", title: "MCP servers", subtitle: "Tool servers", keywords: "mcp servers tools external" },
  { href: "/configuration", title: "Configuration", subtitle: "Daemon configuration", keywords: "config toml configuration permissions" },
  { href: "/skills", title: "Skills", subtitle: "Installed skills and commands", keywords: "skills commands slash catalog" },
  { href: "/import", title: "Import conversations", subtitle: "Bring Claude Code and Codex sessions into Forge", keywords: "import conversations sessions history claude code codex transcripts migrate resume" },
  { href: "/hooks", title: "Hooks", subtitle: "Pre and post tool hooks", keywords: "hooks shell pre post tool" },
  { href: "/session-tree", title: "Session tree", subtitle: "Parent and child sessions", keywords: "tree subagents children fork" },
  { href: "/prs", title: "Pull requests", subtitle: "Open PRs across projects", keywords: "pull requests prs github review merge checks" },
  { href: "/notes", title: "Notes", subtitle: "Scratchpad and to-dos per project", keywords: "notes todo to-do scratchpad checklist project thread" },
  { href: "/remote-access", title: "Remote access", subtitle: "Pair devices and manage who can connect", keywords: "remote access pair qr devices clients revoke token viewer scopes lan tailscale tunnel" },
  { href: "/workflows", title: "Workflows", subtitle: "Workflow runs", keywords: "workflows runs graph" },
];
