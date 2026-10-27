// Wire types for the daemon's /api/onboarding/* routes (crates/forge-cli/src/serve_onboarding.rs),
// mirroring forge-onboarding's serde output verbatim (snake_case).

export type StepId =
  | "install_cli"
  | "welcome"
  | "models"
  | "behavior"
  | "web_search"
  | "benchmarks"
  | "import"
  | "service"
  | "anywhere"
  | "surfaces"
  | "preferences"
  | "reporting"
  | "done";

export type StepStatus = "pending" | "done" | "skipped";
export type VerifyStatus = "verified" | "unchecked" | "rejected";
export type PlanMode = "full" | "resume" | "whats_new";

export interface PlanStep {
  id: StepId;
  title: string;
  summary: string;
  required: boolean;
  status: StepStatus;
  new: boolean;
  satisfied: boolean;
}

export interface Verification {
  status: VerifyStatus;
  at: number;
  detail: string | null;
}

export interface OnboardingState {
  schema: number;
  flow_version: number;
  deferred_count: number;
  steps: Record<string, { status: StepStatus; at: number | null }>;
  verified: Record<string, Verification>;
  required_complete: { tui: boolean; desktop: boolean };
  completed_at: number | null;
  updated_at: number;
}

export type Login = "signed_in" | "signed_out" | "unknown";

export interface BridgeFacts {
  id: string;
  name: string;
  binary: string;
  installed: boolean;
  version: string | null;
  login: Login;
  token_stored: boolean;
  main_login: boolean;
}

export interface KeyFacts {
  id: string;
  name: string;
  configured: boolean;
  from_env: boolean;
  free_tier: boolean;
  key_url: string | null;
  featured: boolean;
}

export interface Machine {
  os: string;
  arch: string;
  forge: { installed: boolean; version: string; exe: string | null };
  bridges: BridgeFacts[];
  oauth: { id: string; name: string; signed_in: boolean }[];
  keys: KeyFacts[];
  local: {
    ollama_installed: boolean;
    ollama_serving: boolean;
    models: string[];
    memory_gb: number;
    recommended: { tag: string; label: string; memory_gb: number } | null;
  };
  search: { brave_key: boolean; searxng_running: boolean; docker: boolean };
  benchmarks: { own_key: boolean };
  imports: {
    claude_dir: boolean;
    codex_dir: boolean;
    cursor_dir: boolean;
    commands: number;
    skills: number;
    mcp_servers: number;
    keybinds_active: boolean;
  };
  settings: {
    permission: string;
    credit_mode: string;
    keybind_preset: string;
    notifications: boolean;
    reporting: string;
    reporting_chosen: boolean;
    theme: string;
  };
  service: { supported: boolean; installed: boolean; running: boolean };
  anywhere: { signed_in: boolean };
  desktop: { supported: boolean; installed: boolean };
  git: { name: string | null; email: string | null };
  project: { cwd: string; git_repo: boolean };
  timed_out: string[];
  took_ms: number;
}

export type SourceKind =
  | "claude_code"
  | "codex"
  | "antigravity"
  | "codex_oauth"
  | "xai_oauth"
  | "api_key"
  | "local";
export type SourceState = "ready" | "needs_login" | "needs_setup" | "not_installed";

export interface ModelSource {
  id: string;
  name: string;
  kind: SourceKind;
  state: SourceState;
  detail: string;
  free_tier: boolean;
  key_url: string | null;
}

export type SecretSlot = "claude_oauth_token" | "brave" | "artificial_analysis";

/** forge_onboarding::Action, serde `tag = "action"`. Secret fields are plain strings on the way
 * in and come back as `***`. */
export type Action =
  | { action: "store_key"; provider: string; key: string; replace?: boolean }
  | { action: "store_secret"; slot: SecretSlot; value: string }
  | { action: "verify"; source: string }
  | { action: "set_behavior"; permission: string; credit_mode: string }
  | { action: "set_keybind_preset"; preset: "forge" | "claude-code" }
  | { action: "set_notifications"; enabled: boolean }
  | { action: "set_theme"; theme: "auto" | "dark" | "light" }
  | { action: "set_local_model"; tag: string; autostart: boolean }
  | { action: "set_reporting"; mode: "auto" | "ask" | "off" }
  | { action: "connect_claude" }
  | { action: "codex_oauth" }
  | { action: "xai_oauth" }
  | { action: "ollama_install" }
  | { action: "ollama_pull"; tag: string }
  | { action: "import_config"; tool: "claude" | "codex" | "cursor" }
  | { action: "import_mcp" }
  | { action: "install_service"; exposure: "local" | "lan" }
  | { action: "anywhere_setup" }
  | { action: "install_desktop" }
  | { action: "start_searxng" };

export interface Recommended {
  step: StepId;
  title: string;
  action: Action | null;
  needs_you: boolean;
}

export interface Snapshot {
  surface: "desktop";
  open?: boolean;
  hint?: string;
  mode: PlanMode;
  complete: boolean;
  missing: StepId[];
  flow_version: number;
  start_index: number;
  plan: PlanStep[];
  state: OnboardingState;
  machine: Machine;
  sources: ModelSource[];
  recommended: Recommended[];
  claude_reconnect: boolean;
}

export interface Brief {
  open: boolean;
  hint?: string;
  complete: boolean;
  flow_version: number;
  claude_reconnect: boolean;
}

export type PromptKind =
  | { kind: "open_url"; url: string }
  | { kind: "device_code"; url: string; code: string }
  | { kind: "terminal" }
  | { kind: "code"; label: string };

export type ActionResult =
  | { kind: "verified"; source: string; status: VerifyStatus; detail: string | null }
  | { kind: "changed" }
  | { kind: "paste_needed"; slot: SecretSlot };

export type JobEvent =
  | { type: "started"; job: number; action: string }
  | { type: "output"; job: number; line: string }
  | { type: "progress"; job: number; label: string; fraction: number | null }
  | { type: "prompt"; job: number; prompt: PromptKind; message: string }
  | { type: "done"; job: number; summary: string; result: ActionResult | null }
  | { type: "failed"; job: number; error: string };

export type JobStateName = "running" | "succeeded" | "failed";

export interface JobPoll {
  job: number;
  action: string;
  state: JobStateName;
  events: JobEvent[];
  next: number;
}

export interface Started {
  job: number;
  label: string;
}
