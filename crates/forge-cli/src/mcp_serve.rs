//! `forge mcp-serve` — an MCP **server** (stdio) that exposes Forge's tool registry to an
//! external agent CLI (claude/codex) so the subscription model runs the **Forge harness**:
//! Forge's own tools, gated by Forge's permission engine (the builtin safety denylist +
//! configured rules). This is Phase 2 of RFC cli-bridge-full-harness — the CLI bridge spawns
//! `forge mcp-serve` and restricts the model to `mcp__forge__*`, so every tool call lands here.
//!
//! Permission: each call runs `permission::decide` before executing; a `Deny` (e.g. the
//! `rm -rf`/secret-read denylist) returns an MCP tool error the model sees and adapts to.
//! Interactive `Ask` is auto-allowed in this non-interactive context (the bridge can't prompt
//! mid-run) — the unoverridable denylist still protects. Forge never sees the CLI's auth.
//!
//! Subagents (RFC subagent-orchestration Phase 3): when subagents are enabled this server also
//! exposes the `spawn_agents` virtual tool, so a subscription model can fan work out to
//! mesh-routed child agents that run in *this* process (it builds its own provider/router/store).
//! Two cross-process mechanisms ride env vars inherited forge → claude/codex → mcp-serve:
//! - `FORGE_SUBAGENT_DEPTH` bounds recursion: this server advertises `spawn_agents` only while
//!   `depth < max_depth`, and bumps the var for anything it spawns (Phase 3c).
//! - `FORGE_SUBAGENT_SINK` names a JSONL file we append child lifecycle to, which the parent
//!   Forge process tails so bridge-spawned subagents are visible in the TUI (Phase 3c).

use std::sync::Arc;

use anyhow::Result;
use forge_config::Config;
use forge_core::hooks;
use forge_core::permission;
use forge_core::subagent::{self, AgentCtx};
use forge_mesh::pricing::Pricing;
use forge_mesh::BudgetState;
use forge_store::Store;
use forge_tools::ToolRegistry;
use forge_types::{PermissionDecision, PermissionMode, PermissionRule};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ContentBlock, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::io::stdio;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

/// Env var holding the bearer token required for the HTTP transport. Stdio is unaffected (it's a
/// trusted parent↔child pipe); HTTP is network-reachable, so it must be behind auth.
const MCP_SERVE_TOKEN_ENV: &str = "FORGE_MCP_SERVE_TOKEN";

/// Bridge-side byte cap on a `read_file` result. Mirrors the native caps claude/codex apply to
/// their own read tools (they page files in ~small chunks); forge-tools' direct-path cap is
/// 256 KiB, which a bridged CLI re-ingests on every subsequent turn of ITS loop — a large read
/// through the bridge costs 2-4x the tokens of the same read in the native CLI. mcp-serve path
/// only; the direct-API path is untouched.
const BRIDGE_READ_CAP_BYTES: usize = 32 * 1024;

/// Bridge-side byte cap on a `shell` result. Mirrors the ~10-16 KiB head+tail clamp claude/codex
/// apply to their own shell tools. mcp-serve path only.
const BRIDGE_SHELL_CAP_BYTES: usize = 16 * 1024;

/// The cap + re-request advice for a tool's result on the bridge path, or `None` for uncapped
/// tools.
fn bridge_cap_for(tool: &str) -> Option<(usize, &'static str)> {
    match tool {
        "read_file" => Some((
            BRIDGE_READ_CAP_BYTES,
            "re-request just the lines you need with start_line/end_line",
        )),
        "shell" => Some((
            BRIDGE_SHELL_CAP_BYTES,
            "re-run with a narrower command or filters (grep/head/tail)",
        )),
        _ => None,
    }
}

/// Clamp an oversized bridge tool result to its cap, keeping the head (headers/signatures) and
/// tail (totals/trailing errors) around an explicit marker so the model knows the middle is
/// missing and how to get it back.
fn cap_bridge_result(tool: &str, text: String) -> String {
    let Some((cap, advice)) = bridge_cap_for(tool) else {
        return text;
    };
    if text.len() <= cap {
        return text;
    }
    let head_end = floor_char_boundary(&text, cap * 3 / 4);
    let tail_start = ceil_char_boundary(&text, text.len() - cap / 4);
    format!(
        "{}\n[… {} of {} bytes omitted by the Forge bridge ({} KiB cap) — {advice} …]\n{}",
        &text[..head_end],
        tail_start - head_end,
        text.len(),
        cap / 1024,
        &text[tail_start..]
    )
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Env var enabling the lean bridge tool surface (same effect as `mesh.bridge_lean = true`).
const BRIDGE_LEAN_ENV: &str = "FORGE_BRIDGE_LEAN";

/// Tools dropped from the advertised list in lean mode: every tool schema/description is
/// re-ingested by the bridged CLI on every turn of ITS loop, so rarely-used surface is a
/// per-instance token tax. The core coding surface (read/write/edit/shell/search/…) stays.
const LEAN_DROPPED_TOOLS: &[&str] = &[
    "web_fetch",
    "web_search",
    "spawn_agents",
    "send_to_agent",
    "remember",
    "present_plan",
];

/// Hard cap (bytes) on the `use_skill` description advertised to a bridged CLI. The full skill
/// catalog (name + description each) reaches several KiB on a skill-heavy machine and is resent
/// every bridge-loop turn.
const BRIDGE_USE_SKILL_DESC_CAP: usize = 1536;

/// Names-only `use_skill` description for the bridge, truncated to [`BRIDGE_USE_SKILL_DESC_CAP`].
fn bridge_use_skill_description(skills: &forge_skills::Catalog) -> String {
    let names: Vec<String> = skills.skill_listing().into_iter().map(|(n, _)| n).collect();
    names_only_skill_description(&names)
}

fn names_only_skill_description(names: &[String]) -> String {
    let base = "Load a Forge skill's methodology into this turn, then follow it. These are \
                Forge's OWN skills — do NOT search the filesystem (~/.claude, ~/.codex) for \
                skills; call this tool with the exact skill name. Available: ";
    let mut out = base.to_string();
    for (i, name) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        let more = format!("… (+{} more)", names.len() - i);
        if out.len() + sep.len() + name.len() + more.len() > BRIDGE_USE_SKILL_DESC_CAP {
            out.push_str(&more);
            return out;
        }
        out.push_str(sep);
        out.push_str(name);
    }
    out
}

/// Append one JSON record to the out-of-band subagent sink the CLI bridge tails (if it gave us
/// one via `FORGE_SUBAGENT_SINK`). Used to surface bridge-turn activity (subagents, task-list
/// updates) in the parent Forge TUI live. Best-effort: no sink / write error is silently ignored.
fn report_to_sink(record: serde_json::Value) {
    let Ok(path) = std::env::var(forge_provider::SUBAGENT_SINK_ENV) else {
        return;
    };
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{record}");
        let _ = f.flush();
    }
}

/// Everything a `spawn_agents` call needs, built once if subagents are enabled here. `ctx`
/// already carries the loaded agent types, the nesting depth, and `max_depth`.
struct SubagentSupport {
    ctx: AgentCtx,
    parent_id: String,
    max_agents: usize,
    max_concurrency: usize,
}

struct ForgeMcp {
    registry: ToolRegistry,
    mode: PermissionMode,
    rules: Vec<PermissionRule>,
    config: Config,
    /// Present when subagents are enabled here (subagents on + `depth < max_depth`).
    subagents: Option<SubagentSupport>,
    /// Store used by the `update_tasks` virtual tool to persist the bridge turn's task list,
    /// keyed by the parent session id the parent's `run_turn` exported via `ENV_SESSION`.
    tasks_store: Arc<Store>,
    /// External MCP servers (mcp-client.md). On the CLI-bridge path the model's tool surface IS
    /// this server, so the MCP meta-tools must be advertised + handled here — otherwise a bridge
    /// model (claude/codex) can't see or call any external MCP tool. `None` when none configured.
    mcp: Option<Arc<forge_mcp::McpManager>>,
    /// Command/skill catalog so a bridged model can discover + load Forge's own skills via the
    /// `use_skill` tool (otherwise claude/codex hunt their own ~/.claude / ~/.codex skills).
    skills: Arc<forge_skills::Catalog>,
    /// Lean tool surface (`FORGE_BRIDGE_LEAN=1` / `mesh.bridge_lean`): drop the tools in
    /// [`LEAN_DROPPED_TOOLS`] from the advertised list.
    lean: bool,
}

impl ServerHandler for ForgeMcp {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.instructions = Some(
            "Forge's own tools (read_file, write_file, edit_file, list_dir, search, shell), \
             gated by Forge's permission engine."
                .into(),
        );
        info
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tool_list(),
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);

        // The subagent virtual tool — fan out to mesh-routed children in this process.
        if name == subagent::SPAWN_AGENTS_TOOL {
            return Ok(self.handle_spawn_agents(&args).await);
        }
        // Follow-ups to a persisted child (persistent subagents) — bridge parity with the
        // direct path's Session::send_to_agent.
        if name == subagent::SEND_TO_AGENT_TOOL {
            return Ok(self.handle_send_to_agent(&args).await);
        }

        // Task tracking — persist the list to the parent session (id from ENV_SESSION) so the
        // parent's run_turn reloads it. Also report it to the out-of-band sink so the parent TUI's
        // sticky task panel updates LIVE during the bridge turn (not just on completion).
        if name == forge_core::UPDATE_TASKS_TOOL {
            let tasks = forge_core::parse_tasks(&args);
            let done = tasks
                .iter()
                .filter(|t| t.status == forge_types::TodoStatus::Done)
                .count();
            if let Ok(session_id) = std::env::var(forge_core::snapshot::ENV_SESSION) {
                let _ = self.tasks_store.set_tasks(&session_id, &tasks);
            }
            report_to_sink(serde_json::json!({ "k": "tasks", "tasks": tasks }));
            return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "task list updated: {} task(s) — {done} done",
                tasks.len()
            ))]));
        }

        // On-demand memory write — bridge model can persist a durable fact mid-turn (parity with
        // the direct path). No embedding here (bridge-side is sync); keyword recall still works.
        if name == forge_core::REMEMBER_TOOL {
            let kind_raw = args.get("kind").and_then(|v| v.as_str()).unwrap_or("fact");
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let kind_norm = kind_raw.trim().to_lowercase();
            let kind_cat = match kind_norm.as_str() {
                "preference" | "decision" | "fact" | "reference" => kind_norm.clone(),
                _ => "fact".to_string(),
            };
            if text.len() < 4 {
                return Ok(CallToolResult::error(vec![ContentBlock::text(
                    "error: memory text too short (minimum 4 characters)",
                )]));
            }
            let scope = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "global".to_string());
            let session_id = std::env::var(forge_core::snapshot::ENV_SESSION)
                .unwrap_or_else(|_| "bridge".to_string());
            match forge_core::embed_one(&self.config.lattice.embeddings, &text).await {
                Some(emb) => {
                    let _ = self.tasks_store.add_memory_with_embedding(
                        &scope,
                        &kind_cat,
                        &text,
                        &session_id,
                        &emb,
                    );
                }
                None => {
                    let _ = self
                        .tasks_store
                        .add_memory(&scope, &kind_cat, &text, &session_id);
                }
            }
            report_to_sink(serde_json::json!({ "k": "memory", "kind": kind_cat, "text": text }));
            return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "memory saved: [{kind_cat}] {text}"
            ))]));
        }

        // Plan presentation — report the plan to the out-of-band sink so the parent renders the
        // card and runs the approval flow (which persists + seeds tasks). The parent ignores it
        // unless it's in planning mode, so building/normal turns are unaffected.
        if name == forge_core::PRESENT_PLAN_TOOL {
            let plan = forge_core::parse_plan(&args);
            let n = plan.steps.len();
            report_to_sink(serde_json::json!({ "k": "plan", "plan": plan }));
            return Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "Plan ({n} step(s)) presented to the user for approval. STOP now — do NOT start \
                 implementing. If the user approves, you'll be switched to Auto-edit and asked to \
                 build it."
            ))]));
        }

        // Skill loading — return the named Forge skill's methodology so the bridge model applies
        // it (parity with the direct path's use_skill handler).
        if name == forge_core::USE_SKILL_TOOL {
            let skill = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            return Ok(match self.skills.skill_guidance(skill) {
                Some(g) => CallToolResult::success(vec![ContentBlock::text(format!(
                    "Loaded the '{skill}' skill. Apply this methodology now:\n\n{g}"
                ))]),
                None => {
                    let available = self
                        .skills
                        .skill_listing()
                        .into_iter()
                        .map(|(n, _)| n)
                        .collect::<Vec<_>>()
                        .join(", ");
                    CallToolResult::error(vec![ContentBlock::text(format!(
                        "no Forge skill named '{skill}'. Available: {available}"
                    ))])
                }
            });
        }

        // External MCP meta-tools — gate (External/ReadOnly) then route to the manager. Server
        // tools are invoked via `mcp_call`, so this covers the whole external surface. Hooks fire
        // here too (PreToolUse block + arg-rewrite, PostToolUse observe) so the hook-based
        // permission/logging story applies to MCP traffic, not just built-in tools.
        if let Some(m) = &self.mcp {
            if m.knows_tool(&name) {
                let side_effect = m.side_effect_of(&name);
                let mut effective_args = args.clone();

                // PreToolUse: block, or rewrite the args before the gate + dispatch.
                if !self.config.hooks.is_empty() {
                    let payload =
                        serde_json::json!({ "tool": name, "args": effective_args }).to_string();
                    let outcome = hooks::run_hooks(
                        &self.config.hooks,
                        forge_config::HookEvent::PreToolUse,
                        &name,
                        &payload,
                    )
                    .await;
                    if let Some(reason) = outcome.blocked {
                        return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                            "blocked by hook: {reason}"
                        ))]));
                    }
                    if let Some(rewritten) = outcome.rewritten_args {
                        effective_args = rewritten;
                    }
                }

                let decision =
                    permission::decide(self.mode, side_effect, &name, &effective_args, &self.rules);
                if decision == PermissionDecision::Deny {
                    return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                        "denied by Forge permission policy: {name}"
                    ))]));
                }
                let out = m.call(&name, &effective_args).await;

                // PostToolUse: observe; notes are prefixed onto the bridge model's result text.
                let mut note_prefix = String::new();
                if !self.config.hooks.is_empty() {
                    let payload = serde_json::json!({
                        "tool": name, "args": effective_args, "result": out.text, "ok": out.ok
                    })
                    .to_string();
                    let outcome = hooks::run_hooks(
                        &self.config.hooks,
                        forge_config::HookEvent::PostToolUse,
                        &name,
                        &payload,
                    )
                    .await;
                    for note in outcome.notes {
                        note_prefix.push_str(&format!("[hook note] {note}\n"));
                    }
                }

                let text = if note_prefix.is_empty() {
                    out.text
                } else {
                    format!("{note_prefix}{}", out.text)
                };
                let content = vec![ContentBlock::text(text)];
                return Ok(if out.ok {
                    CallToolResult::success(content)
                } else {
                    CallToolResult::error(content)
                });
            }
        }

        let Some(tool) = self.registry.get(&name) else {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool: {name}"
            ))]));
        };

        // PreToolUse hooks (hooks.md): fire before the permission gate so user hooks can block
        // calls on the CLI-bridge path the same way they do on the direct path.
        if !self.config.hooks.is_empty() {
            let payload = serde_json::json!({ "tool": name, "args": args }).to_string();
            let outcome = hooks::run_hooks(
                &self.config.hooks,
                forge_config::HookEvent::PreToolUse,
                &name,
                &payload,
            )
            .await;
            if let Some(reason) = outcome.blocked {
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "blocked by hook: {reason}"
                ))]));
            }
        }

        // Forge's permission gate — the unoverridable denylist always applies here.
        let decision = permission::decide(self.mode, tool.side_effect(), &name, &args, &self.rules);
        if decision == PermissionDecision::Deny {
            return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "denied by Forge permission policy: {name}"
            ))]));
        }

        // Snapshot the target's pre-edit bytes into the parent turn's checkpoint dir (read from
        // the env the parent's run_turn exported), so files a bridge model edits are restorable by
        // `/undo` exactly like in-process edits (RFC PR3, cross-process). No-op if unset.
        let write_path = (tool.side_effect() == forge_types::SideEffect::Write)
            .then(|| args.get("path").and_then(|v| v.as_str()))
            .flatten()
            .map(std::path::PathBuf::from);
        if let Some(path) = &write_path {
            let _ = forge_core::snapshot::snapshot_from_env_before_write(path);
        }

        let (out, ok) = match tool.run(&args).await {
            Ok(out) => {
                if let Some(path) = &write_path {
                    let _ = forge_core::snapshot::record_from_env_after_write(path);
                }
                (cap_bridge_result(&name, out), true)
            }
            Err(e) => (format!("error: {e}"), false),
        };

        // PostToolUse hooks (hooks.md): observe the completed call. Notes are surfaced as a
        // prefix on the result the bridge model sees (no presenter available on this path).
        let mut note_prefix = String::new();
        if !self.config.hooks.is_empty() {
            let payload =
                serde_json::json!({ "tool": name, "args": args, "result": out, "ok": ok })
                    .to_string();
            let outcome = hooks::run_hooks(
                &self.config.hooks,
                forge_config::HookEvent::PostToolUse,
                &name,
                &payload,
            )
            .await;
            for note in outcome.notes {
                note_prefix.push_str(&format!("[hook note] {note}\n"));
            }
        }

        let result_text = if note_prefix.is_empty() {
            out
        } else {
            format!("{note_prefix}{out}")
        };
        Ok(if ok {
            CallToolResult::success(vec![ContentBlock::text(result_text)])
        } else {
            CallToolResult::error(vec![ContentBlock::text(result_text)])
        })
    }
}

impl ForgeMcp {
    /// The advertised tool list. Factored out of `list_tools` (which needs an rmcp
    /// `RequestContext`) so the lean-surface behavior is unit-testable.
    fn tool_list(&self) -> Vec<Tool> {
        let mut tools: Vec<Tool> = self
            .registry
            .names()
            .filter_map(|name| self.registry.get(name))
            .map(|t| {
                let schema: JsonObject = t.schema().as_object().cloned().unwrap_or_default();
                Tool::new(
                    t.name().to_string(),
                    t.description().to_string(),
                    Arc::new(schema),
                )
            })
            .collect();
        // Advertise the subagent virtual tools only when enabled here — spawn_agents for fresh
        // fan-out, send_to_agent for follow-ups to persisted children (persistent subagents).
        if let Some(s) = &self.subagents {
            let spec = subagent::spawn_agents_spec(s.max_agents);
            let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
            tools.push(Tool::new(spec.name, spec.description, Arc::new(schema)));
            let follow = subagent::send_to_agent_spec();
            let follow_schema: JsonObject = follow.schema.as_object().cloned().unwrap_or_default();
            tools.push(Tool::new(
                follow.name,
                follow.description,
                Arc::new(follow_schema),
            ));
        }
        // Always advertise task tracking so a bridge model can maintain a visible todo list. The
        // description is bridge-specific: the direct path's spec encourages an update per status
        // change, which through a bridged CLI means a whole extra model round-trip per task — so
        // here it asks for batched updates instead.
        let ts = forge_core::update_tasks_spec();
        let ts_schema: JsonObject = ts.schema.as_object().cloned().unwrap_or_default();
        tools.push(Tool::new(
            ts.name,
            "Create the task list once up front. Batch status changes — update in one call when \
             several steps changed. One final call marking remaining tasks Done before finishing."
                .to_string(),
            Arc::new(ts_schema),
        ));
        // Always advertise the on-demand memory tool so a bridge model can persist facts mid-turn.
        let rs = forge_core::remember_spec();
        let rs_schema: JsonObject = rs.schema.as_object().cloned().unwrap_or_default();
        tools.push(Tool::new(rs.name, rs.description, Arc::new(rs_schema)));
        // Advertise plan presentation so a bridge model can propose a plan in planning mode. The
        // bridge can't see the parent's runtime temper, so it's advertised unconditionally; the
        // parent honors the plan only when it is actually in Plan mode (gated in run_model_loop).
        let ps = forge_core::present_plan_spec();
        let ps_schema: JsonObject = ps.schema.as_object().cloned().unwrap_or_default();
        tools.push(Tool::new(ps.name, ps.description, Arc::new(ps_schema)));
        // Advertise skill loading so a bridge model finds + uses Forge's own skills instead of
        // its native ones. The description is capped to a names-only listing here: the direct
        // path's full catalog (name + description per skill) is re-ingested by the bridged CLI
        // on EVERY turn of its own loop, so it costs far more through the bridge.
        if !self.skills.skill_listing().is_empty() {
            let us = forge_core::use_skill_spec(&self.skills);
            let us_schema: JsonObject = us.schema.as_object().cloned().unwrap_or_default();
            tools.push(Tool::new(
                us.name,
                bridge_use_skill_description(&self.skills),
                Arc::new(us_schema),
            ));
        }
        // External MCP meta-tools (mcp_search_tools / mcp_call / resources / prompt) so a bridge
        // model can discover + call the connected servers' tools (e.g. helm). Empty if none.
        if let Some(m) = &self.mcp {
            for spec in m.advertised_specs() {
                let schema: JsonObject = spec.schema.as_object().cloned().unwrap_or_default();
                tools.push(Tool::new(spec.name, spec.description, Arc::new(schema)));
            }
        }
        if self.lean {
            tools.retain(|t| !LEAN_DROPPED_TOOLS.contains(&t.name.as_ref()));
        }
        tools
    }

    /// Bridge-side `send_to_agent`: resolve the child among the parent session's persisted
    /// children, rebuild its transcript, and continue it — the same semantics as the direct
    /// path's `Session::send_to_agent`, reported through the out-of-band sink instead of a
    /// presenter.
    async fn handle_send_to_agent(&self, args: &Value) -> CallToolResult {
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "send_to_agent is not available here",
            )]);
        };
        let address = args
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let message = args
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if address.is_empty() || message.is_empty() {
            return CallToolResult::error(vec![ContentBlock::text(
                "error: send_to_agent needs both `agent` (name or id prefix) and `message`",
            )]);
        }
        let children = s
            .ctx
            .store
            .named_child_sessions(&s.parent_id)
            .unwrap_or_default();
        let Some((child_id, agent_name)) = subagent::resolve_child_address(&children, &address)
        else {
            let known: Vec<String> = children
                .iter()
                .map(|(id, t)| format!("{} ({})", t.as_deref().unwrap_or("unnamed"), &id[..8]))
                .collect();
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "error: no child agent matches '{address}'. Children this session: [{}] — \
                 spawn one first with spawn_agents",
                known.join(", ")
            ))]);
        };
        let request = subagent::AgentRequest {
            agent: agent_name.clone(),
            task: message.clone(),
        };
        let resolved = subagent::resolve(&request, &s.ctx.agents);
        let budget = BudgetState {
            spent_today_usd: s.ctx.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: s.ctx.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: s.ctx.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };
        let decision = subagent::route_child(&s.ctx, &resolved, budget).await;
        let mut on_delta = |_: forge_provider::StreamEvent| {};
        match subagent::resume_subagent(
            &s.ctx,
            &child_id,
            &resolved,
            &message,
            decision,
            budget,
            &mut on_delta,
        )
        .await
        {
            Ok(outcome) => {
                let label = if agent_name.is_empty() {
                    child_id[..8].to_string()
                } else {
                    agent_name
                };
                let text = format!("[{label}] {}", outcome.final_text);
                if outcome.ok {
                    CallToolResult::success(vec![ContentBlock::text(text)])
                } else {
                    CallToolResult::error(vec![ContentBlock::text(text)])
                }
            }
            Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
                "error: send_to_agent failed: {e}"
            ))]),
        }
    }

    async fn handle_spawn_agents(&self, args: &Value) -> CallToolResult {
        let Some(s) = &self.subagents else {
            return CallToolResult::error(vec![ContentBlock::text(
                "spawn_agents is not available here",
            )]);
        };
        let requests = match subagent::parse_requests(args, s.max_agents) {
            Ok(r) => r,
            Err(msg) => {
                return CallToolResult::error(vec![ContentBlock::text(format!("error: {msg}"))])
            }
        };

        let budget = BudgetState {
            spent_today_usd: s.ctx.store.spend_today_usd().unwrap_or(0.0),
            daily_cap_usd: self.config.mesh.daily_budget_usd,
            spent_week_usd: s.ctx.store.spend_this_week_usd().unwrap_or(0.0),
            weekly_cap_usd: self.config.mesh.weekly_budget_usd,
            spent_month_usd: s.ctx.store.spend_this_month_usd().unwrap_or(0.0),
            monthly_cap_usd: self.config.mesh.monthly_cap_usd,
            warn_fraction: self.config.mesh.warn_threshold,
            min_context_tokens: None,
        };

        // Report subagent lifecycle to the out-of-band sink (if the bridge gave us one) so the
        // parent Forge TUI shows these children natively (RFC subagent-orchestration Phase 3c).
        let mut sink = std::env::var(forge_provider::SUBAGENT_SINK_ENV)
            .ok()
            .and_then(|p| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(p)
                    .ok()
            });
        let mut write = move |v: serde_json::Value| {
            if let Some(f) = sink.as_mut() {
                use std::io::Write;
                let _ = writeln!(f, "{v}");
                let _ = f.flush();
            }
        };
        let mut on_event = |ev: subagent::Lifecycle| match ev {
            subagent::Lifecycle::Start {
                id,
                agent,
                task,
                model,
            } => {
                write(
                    serde_json::json!({"k":"start","id":id,"agent":agent,"task":task,"model":model}),
                );
            }
            subagent::Lifecycle::Progress { id, snippet } => {
                write(serde_json::json!({"k":"progress","id":id,"snippet":snippet}));
            }
            subagent::Lifecycle::Done {
                id,
                agent,
                ok,
                summary,
                cost_usd,
            } => {
                write(
                    serde_json::json!({"k":"done","id":id,"agent":agent,"ok":ok,"summary":summary,"cost":cost_usd}),
                );
            }
        };

        match subagent::orchestrate(
            &s.ctx,
            &s.parent_id,
            requests,
            budget,
            s.max_concurrency,
            &mut on_event,
        )
        .await
        {
            Ok((combined, _ok)) => CallToolResult::success(vec![ContentBlock::text(combined)]),
            Err(e) => {
                CallToolResult::error(vec![ContentBlock::text(format!("subagents failed: {e}"))])
            }
        }
    }
}

/// Run the Forge MCP server until the client disconnects. Loads config from the cwd (so it shares
/// the project's permission rules) and serves the core tool registry. `http=false` serves on stdio
/// (the CLI-bridge default); `http=true` serves the SAME tool surface over streamable-HTTP on
/// `bind`, behind a bearer token (`FORGE_MCP_SERVE_TOKEN`) for remote/multi-machine orchestration.
pub async fn run(http: bool, bind: String) -> Result<()> {
    forge_config::inject_provider_keys();
    let mut config = forge_config::load().unwrap_or_else(|_| Config::default());
    // The parent hands us its CURRENT runtime temper in OUR env, set explicitly on this child's
    // `Command` at the bridge spawn site (`CompletionOptions::checkpoint` → `bridge_mcp_env`).
    // Honor it over the on-disk config mode so the permission gate matches the UI: after the user
    // switches Plan→Auto-edit (e.g. approving a plan), writes are actually allowed here — previously
    // the bridge used the stale config mode and denied them, which the model reported as "no perms".
    if let Some(mode) = std::env::var(forge_core::snapshot::ENV_MODE)
        .ok()
        .and_then(|s| PermissionMode::from_key(&s))
    {
        config.permission_mode = mode;
    }

    // Current nesting depth, carried across the process boundary. Subagents are exposed here
    // only when enabled AND there is depth budget left — a nested bridge child inherits a
    // higher FORGE_SUBAGENT_DEPTH and stops advertising the tool once it reaches max_depth.
    let depth: usize = std::env::var(crate::FORGE_SUBAGENT_DEPTH_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // Publish depth+1 for any bridge subprocess our own children spawn (they inherit OUR env via
    // `bridge_mcp_env`'s fallback read). Depth is a per-PROCESS constant, so write it exactly
    // once HERE — during single-threaded init, before the server starts handling anything —
    // never per-request: `handle_spawn_agents` used to `set_var` on every call, and a process-
    // global env write racing a concurrent `getenv` on another thread is undefined behavior
    // (the same hazard class the explicit `CheckpointContext` threading removed for
    // FORGE_CHECKPOINT_*).
    std::env::set_var(crate::FORGE_SUBAGENT_DEPTH_ENV, (depth + 1).to_string());
    let subagents = if config.mesh.subagents.enabled && depth < config.mesh.subagents.max_depth {
        // Same global store the parent uses — NOT a relative `.forge/forge.db`, which is a DIFFERENT
        // file (the parent's store lives in the per-user data dir). The divergence created spurious
        // empty sessions and broke the bridge task round-trip (the parent reloaded tasks from the
        // global db but mcp-serve wrote them to the project-local one).
        let store = Arc::new(crate::open_store()?);
        let repo_key = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let repo_boosts = store.duel_boosts(&repo_key).unwrap_or_default();
        let (provider, router) = crate::build_provider_and_router(
            &config,
            false,
            None,
            None,
            Default::default(),
            repo_boosts,
        );
        let parent_id = store.create_session(".", &format!("{:?}", config.permission_mode))?;
        let agents = Arc::new(forge_config::load_agents(std::path::Path::new(
            &config.mesh.subagents.agents_dir,
        )));
        let ctx = AgentCtx {
            provider,
            router,
            store,
            config: config.clone(),
            pricing: Pricing::from_config(&config),
            mode: config.permission_mode,
            rules: config.permission_rules(),
            depth,
            max_depth: config.mesh.subagents.max_depth,
            agents,
            worktree_root: None,
            repo_root: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        };
        Some(SubagentSupport {
            ctx,
            parent_id,
            max_agents: config.mesh.subagents.max_agents,
            max_concurrency: config.mesh.subagents.max_concurrency,
        })
    } else {
        None
    };

    // Reuse the subagent store if present, else open the SAME global store the parent uses, so the
    // bridge turn's `update_tasks` persists where the parent's post-turn reload reads it.
    let tasks_store = match &subagents {
        Some(s) => Arc::clone(&s.ctx.store),
        None => Arc::new(crate::open_store()?),
    };
    // Connect the external MCP servers in THIS process so the bridge model can drive them — the
    // bridge's whole tool surface is this server. Skipped (None) when none are configured.
    // Connect external MCP servers in the BACKGROUND so the bridge serves Forge's own tools
    // (update_tasks, spawn_agents, file/shell, …) IMMEDIATELY. Awaiting connect here previously
    // stalled the whole tool list behind a slow/auth-gated server (e.g. helm), so the spawned
    // claude/codex CLI timed out waiting for the MCP server and fell back to its native tools
    // ("update_tasks not in my toolset"). The meta-tools are advertised right away via
    // `connecting`; the first `mcp_call` lazily connects on demand.
    let mcp = if config.mcp.active_servers().next().is_some() {
        let mgr = Arc::new(forge_mcp::McpManager::connecting(&config.mcp));
        let bg = Arc::clone(&mgr);
        tokio::spawn(async move { bg.connect_active().await });
        Some(mgr)
    } else {
        None
    };
    let skills = Arc::new(forge_skills::Catalog::load(&forge_config::command_sources()));
    let lean = std::env::var(BRIDGE_LEAN_ENV)
        .map(|v| v == "1")
        .unwrap_or(false)
        || config.mesh.bridge_lean;
    let server = ForgeMcp {
        registry: ToolRegistry::with_core_tools(),
        mode: config.permission_mode,
        rules: config.permission_rules(),
        config,
        subagents,
        tasks_store,
        mcp,
        skills,
        lean,
    };
    if http {
        return serve_http(server, &bind).await;
    }
    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

/// Serve the same `ForgeMcp` tool surface over rmcp's streamable-HTTP server transport, mounted in
/// axum behind a bearer-token gate. One shared `ForgeMcp` backs every session (its handlers take
/// `&self`), so the rmcp session factory just hands out `Arc` clones.
async fn serve_http(server: ForgeMcp, bind: &str) -> Result<()> {
    let token = std::env::var(MCP_SERVE_TOKEN_ENV)
        .ok()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "HTTP transport requires an auth token — set {MCP_SERVE_TOKEN_ENV} to a secret \
                 bearer token before running `forge mcp-serve --transport http`."
            )
        })?;

    let shared = Arc::new(server);
    let service = StreamableHttpService::new(
        move || Ok(Arc::clone(&shared)),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );

    let app = axum::Router::new().nest_service("/mcp", service).layer(
        axum::middleware::from_fn_with_state(Arc::new(token), auth_middleware),
    );

    let listener = tokio::net::TcpListener::bind(bind).await?;
    let local = listener.local_addr()?;
    eprintln!("forge mcp-serve: streamable-HTTP transport on http://{local}/mcp (bearer auth)");
    axum::serve(listener, app).await?;
    Ok(())
}

/// axum middleware: reject any request whose `Authorization` header isn't `Bearer <token>` matching
/// `FORGE_MCP_SERVE_TOKEN`. Applied to the whole MCP router so unauthenticated peers never reach a
/// tool.
async fn auth_middleware(
    axum::extract::State(token): axum::extract::State<Arc<String>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let provided = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if bearer_ok(provided, &token) {
        next.run(req).await
    } else {
        (axum::http::StatusCode::UNAUTHORIZED, "unauthorized\n").into_response()
    }
}

/// Whether an `Authorization` header value is a `Bearer <token>` that matches `expected`. The token
/// comparison is length-checked then constant-time to avoid leaking it via response timing.
fn bearer_ok(header: Option<&str>, expected: &str) -> bool {
    let Some(rest) = header.and_then(|h| {
        h.strip_prefix("Bearer ")
            .or_else(|| h.strip_prefix("bearer "))
    }) else {
        return false;
    };
    let provided = rest.trim().as_bytes();
    let expected = expected.as_bytes();
    if provided.len() != expected.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in provided.iter().zip(expected.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_bridge_result_passes_small_and_uncapped_tools_through() {
        let small = "x".repeat(1000);
        assert_eq!(cap_bridge_result("read_file", small.clone()), small);
        let big = "y".repeat(BRIDGE_READ_CAP_BYTES + 1000);
        assert_eq!(
            cap_bridge_result("write_file", big.clone()),
            big,
            "only read_file/shell are capped"
        );
    }

    #[test]
    fn cap_bridge_result_clamps_read_with_head_tail_and_marker() {
        let text = format!("HEAD{}TAIL", "m".repeat(BRIDGE_READ_CAP_BYTES * 2));
        let capped = cap_bridge_result("read_file", text.clone());
        assert!(capped.len() < text.len());
        assert!(
            capped.len() <= BRIDGE_READ_CAP_BYTES + 256,
            "cap + marker only"
        );
        assert!(capped.starts_with("HEAD"));
        assert!(capped.ends_with("TAIL"));
        assert!(capped.contains("omitted by the Forge bridge"));
        assert!(capped.contains("start_line/end_line"));
    }

    #[test]
    fn cap_bridge_result_clamps_shell_with_shell_advice() {
        let text = "z".repeat(BRIDGE_SHELL_CAP_BYTES * 3);
        let capped = cap_bridge_result("shell", text);
        assert!(capped.len() <= BRIDGE_SHELL_CAP_BYTES + 256);
        assert!(capped.contains("narrower command"));
    }

    #[test]
    fn cap_bridge_result_is_multibyte_safe() {
        let text = "é".repeat(BRIDGE_READ_CAP_BYTES); // 2 bytes each → over cap, odd boundaries
        let capped = cap_bridge_result("read_file", text);
        assert!(capped.contains("omitted by the Forge bridge"));
    }

    fn test_server(lean: bool) -> ForgeMcp {
        let config = Config::default();
        ForgeMcp {
            registry: ToolRegistry::with_core_tools(),
            mode: config.permission_mode,
            rules: Vec::new(),
            config,
            subagents: None,
            tasks_store: Arc::new(Store::open_in_memory().unwrap()),
            mcp: None,
            skills: Arc::new(forge_skills::Catalog::load(
                &forge_skills::Sources::default(),
            )),
            lean,
        }
    }

    #[test]
    fn full_surface_advertises_web_and_ritual_tools() {
        let names: Vec<String> = test_server(false)
            .tool_list()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for expected in [
            "read_file",
            "shell",
            "web_fetch",
            "web_search",
            "remember",
            "present_plan",
            "update_tasks",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn lean_surface_drops_heavy_tools_keeps_core() {
        let names: Vec<String> = test_server(true)
            .tool_list()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for dropped in LEAN_DROPPED_TOOLS {
            assert!(
                !names.contains(&dropped.to_string()),
                "{dropped} should be dropped"
            );
        }
        for kept in [
            "read_file",
            "write_file",
            "edit_file",
            "shell",
            "search",
            "list_dir",
            "update_tasks",
        ] {
            assert!(names.contains(&kept.to_string()), "{kept} should stay");
        }
    }

    #[test]
    fn names_only_skill_description_is_hard_capped() {
        let names: Vec<String> = (0..200)
            .map(|i| format!("some-long-skill-name-{i}"))
            .collect();
        let desc = names_only_skill_description(&names);
        assert!(
            desc.len() <= BRIDGE_USE_SKILL_DESC_CAP,
            "len={}",
            desc.len()
        );
        assert!(desc.contains("more)"), "truncation marker present");
        assert!(desc.contains("some-long-skill-name-0"));
        let few = vec!["rust".to_string(), "tdd".to_string()];
        let short = names_only_skill_description(&few);
        assert!(short.contains("rust, tdd"));
        assert!(!short.contains("more)"));
    }

    #[test]
    fn bearer_ok_accepts_exact_token_either_case_prefix() {
        assert!(bearer_ok(Some("Bearer s3cret"), "s3cret"));
        assert!(bearer_ok(Some("bearer s3cret"), "s3cret"));
        // Trailing whitespace around the token is tolerated.
        assert!(bearer_ok(Some("Bearer  s3cret "), "s3cret"));
    }

    #[test]
    fn bearer_ok_rejects_bad_or_missing_tokens() {
        assert!(!bearer_ok(None, "s3cret"));
        assert!(!bearer_ok(Some(""), "s3cret"));
        assert!(!bearer_ok(Some("s3cret"), "s3cret")); // missing "Bearer " scheme
        assert!(!bearer_ok(Some("Bearer wrong"), "s3cret"));
        assert!(!bearer_ok(Some("Bearer s3cre"), "s3cret")); // length mismatch
        assert!(!bearer_ok(Some("Basic s3cret"), "s3cret"));
    }

    /// End-to-end (in-memory, no network) check that the auth middleware actually gates a router:
    /// a request without the bearer is 401, and with it passes through to the inner handler.
    #[tokio::test]
    async fn auth_middleware_gates_router() {
        use axum::body::Body;
        use axum::http::{header::AUTHORIZATION, Request, StatusCode};
        use tower::ServiceExt;

        let app = axum::Router::new()
            .route("/", axum::routing::get(|| async { "ok" }))
            .layer(axum::middleware::from_fn_with_state(
                Arc::new("topsecret".to_string()),
                auth_middleware,
            ));

        let unauth = app
            .clone()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

        let authed = app
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(AUTHORIZATION, "Bearer topsecret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(authed.status(), StatusCode::OK);
    }
}
