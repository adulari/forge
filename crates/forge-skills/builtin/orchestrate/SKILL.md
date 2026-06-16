---
name: orchestrate
description: >-
  Forge's universal task router. Evaluates any task against ALL available Forge
  resources — skills, subagents, external MCP tools, web access, the Lattice code
  graph — decomposes it, and routes each part through the best resource(s) in the
  right order. The default entry point for complex, ambiguous, or multi-step work.
tier: complex
---

# Orchestrate — Forge's Universal Router

A built-in Forge feature. Given any task, it discovers what Forge can do **right now**,
decomposes the task, and routes each subtask through the best skill(s), subagent(s),
and tool(s) — sequentially where dependent, in parallel where independent.

**Never rely on a hardcoded list.** Discover resources fresh every time so newly added
skills, MCP servers, and tools are picked up automatically.

---

## Step 1 — Understand the task

- What is the **goal**? What is the **current state**? What are the **constraints**?
- Is it **design**, **implementation**, or **mixed**?

If genuinely ambiguous, ask one clarifying question. Otherwise proceed.

---

## Step 2 — Discover Forge's resources (at runtime)

- **Skills** — call the `use_skill` tool. Its description lists every skill in Forge's
  library by name; that list is authoritative. Do NOT route on a name alone — read the
  description before choosing. Do NOT read the filesystem to find skills.
- **Subagents** — the `spawn_agents` tool fans work out to mesh-routed child agents.
  Use it for long, parallel, or isolation-worthy subtasks.
- **External tools (MCP)** — `mcp_search_tools` discovers tools on connected servers;
  `mcp_call` invokes them. Use for anything needing an external system.
- **Web** — `web_search` and `web_fetch` for live information and URLs.
- **Code intelligence** — the Lattice graph already injects relevant code each turn;
  the `lattice` tool answers structural questions (impact, paths, provenance).

---

## Step 3 — Decompose

Break the task into subtasks, each mapped to a discovered resource. For each:
- **Which resource fits?** Match the subtask to a skill/subagent/MCP tool by its purpose.
- **What does it produce** for downstream steps?
- **Can it run in parallel**, or does it depend on another subtask?

---

## Step 4 — Order the work

Sequential when step N needs step N-1's output (design before implementation, root cause
before fix). Parallel when steps are independent. Sketch the dependency graph before acting.

---

## Step 5 — State the plan, then execute

State briefly: **Task** (what you're doing), **Resources** (which skills/subagents/tools, in
what order), **Rationale** (one line per non-obvious choice). Then execute — don't stop for
approval unless a step needs a decision only the user can make (a value/scope/credential).

Routing is not complete until the resource is actually invoked. Naming a skill is not using
it — call `use_skill`. Describing a fan-out is not running it — call `spawn_agents`. Resist
narrating; invoke the tool.

- **One resource covers it** → invoke it immediately.
- **Several in sequence** → invoke each in order, feeding outputs forward.
- **Independent work** → run in parallel (`spawn_agents`, or several tool calls in one turn).
- **Implementation between design steps** → after a design skill produces its output, proceed
  straight to the edits without waiting for instruction.

---

## Step 6 — Verify completeness

- Was the primary goal achieved? Any gap (a design done but implementation skipped)?
- Did any step surface something that changes the plan?

State what was completed and flag any gaps explicitly.

---

## Choosing a resource type

| Situation | Resource |
|-----------|----------|
| Guiding *how* to approach a task in this conversation | a **skill** (`use_skill`) |
| A long, parallel, or isolation-worthy subtask | a **subagent** (`spawn_agents`) |
| Data from / action in an external system | an **MCP tool** (`mcp_search_tools` → `mcp_call`) |
| Live information or a specific URL | `web_search` / `web_fetch` |
| A structural question about this codebase | the `lattice` tool |

When several skills match, prefer the one whose description most specifically covers the
actual work. Read the descriptions rather than guessing from names.
