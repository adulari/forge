# Forge vs. the AI coding agents — detailed comparison

A sourced, honest breakdown of how Forge compares to the major AI coding agents and CLIs as of
**mid-2026**. This is the long-form companion to the comparison table in the
[README](../README.md#comparison). Every competitor claim here is from a primary source (the tool's
own docs, repo, or pricing page) or flagged as unverified. Corrections welcome — open an issue with a
source.

> Three corporate-status changes happened in 2026 that matter for any comparison:
> - **Gemini CLI was retired for individuals on June 18, 2026**, replaced by the Go-based,
>   closed-source **Antigravity CLI** (the OSS repo persists for enterprise only).
> - **Windsurf became "Devin Desktop"** in June 2026 after Cognition's acquisition; `windsurf.com`
>   redirects to `devin.ai`.
> - **Roo Code shut down May 15, 2026** (archived); the community fork **ZooCode** continues it.
>
> Model version strings (Opus 4.x, GPT-5.x, etc.) shift constantly and are kept out of load-bearing
> claims here.

---

## What makes Forge different

Forge's wedge is **automatic, benchmark-ranked, cost-tiered routing across independent providers, with
cross-provider failover and subscription bridging** — in a single Rust binary. The research below
confirms the core hypothesis:

- **No competitor does automatic cost/benchmark cheapest-capable cross-provider routing.** The closest
  is **GitHub Copilot's "Auto" mode** (documented complexity + model-health routing with cross-vendor
  failover) — but only within GitHub's own hosted catalog, not an open multi-provider marketplace, and
  not benchmark/cost-optimized in the sense Forge means. Cursor "Auto" and Windsurf "Adaptive" are
  vague balance-selectors with no published cost logic.
- **Native cross-provider auto-failover** exists only in Copilot's auto mode (in-catalog). Everyone
  else: no, or undocumented.
- **Running someone else's consumer subscription** (Claude Pro/Max, ChatGPT Plus) through the agent is
  either single-vendor (Claude Code, Codex CLI use only their own) or actively blocked by Anthropic's
  2026 ToS enforcement (Cline wrapper, opencode). Forge bridges Claude Code, Codex, and Antigravity
  (free Gemini) and layers routing + failover on top.

A multi-provider **Rust** CLI with auto mesh routing + failover has no direct equivalent. Codex CLI is
Rust and multi-provider-capable but does no routing or failover; the OSS multi-provider field (Aider,
Cline, opencode) is Python/TypeScript with manual model selection.

Beyond routing, several Forge capabilities have **no equivalent in any tool below**, precisely
*because* they need a multi-model mesh underneath: `/duel` (race distinct-provider models on one
task in parallel worktrees, merge the winner, and feed the outcome back into routing),
`forge blame` (per-line provenance — which model, session, and prompt wrote a line), sandboxed JS
workflow scripts with mesh-routed `agent()` fan-out, and `forge schedule` (recurring headless runs
on native OS timers, no daemon).

---

## Per-tool breakdown

### Claude Code (Anthropic)
- **Models / providers:** Claude models only, served via Anthropic API, Amazon Bedrock, or Google
  Vertex — multiple *hosting platforms* but a single *vendor*. Non-Claude models only through
  third-party gateway shims.
- **Routing / failover:** No native cost routing; no cross-provider failover.
- **Subscription:** Yes — runs on Claude Pro / Max (its own plans).
- **Local LLMs:** Not native (proxy / env-var workarounds only).
- **Extensibility:** MCP client ✅, MCP server ✅, plugins/marketplace ✅, custom slash commands ✅,
  subagents + hooks ✅.
- **Open source:** No — proprietary ("All rights reserved" in the official LICENSE). Runtime:
  Node.js / TypeScript (`@anthropic-ai/claude-code`).
- **Pricing:** Pro $20, Max 5× $100, Max 20× $200/mo; or API pay-as-you-go.
- **Sources:** code.claude.com/docs, raw.githubusercontent.com/anthropics/claude-code/main/LICENSE.md,
  claude.com/pricing

### OpenAI Codex CLI
- **Models / providers:** OpenAI GPT-5.x family **plus custom OpenAI-compatible providers** via
  `config.toml` (built-in IDs include `openai`, `ollama`, `lmstudio`). The most multi-provider of the
  big-vendor CLIs. Caveat: may be constrained to the Responses API (`wire_api = "responses"`) on some
  versions — verify per release.
- **Routing / failover:** No automatic routing; no cross-provider failover.
- **Subscription:** Yes — runs on a ChatGPT plan (its own).
- **Local LLMs:** Yes — Ollama / LM Studio built-in.
- **Extensibility:** MCP client ✅, MCP server ✅ (`codex mcp-server`), plugins/marketplace ✅,
  `AGENTS.md` ✅, slash commands via skills.
- **Open source:** **Yes, Apache-2.0, Rust** (rewritten from the original TypeScript version).
- **Pricing:** Free $0, Go $8, Plus $20, Pro from $100/mo; or API pay-as-you-go.
- **Sources:** github.com/openai/codex, developers.openai.com/codex/config-reference,
  developers.openai.com/codex/pricing

### Gemini CLI (Google) — retired for individuals (June 18, 2026)
- **Models / providers:** Gemini only; no official non-Gemini support.
- **Routing / failover:** No; same-vendor Pro→Flash fallback only (reported buggy).
- **Subscription:** Was yes (Google OAuth free tier) until June 18, 2026; now paid API keys / Code
  Assist only.
- **Local LLMs:** No.
- **Extensibility:** MCP client ✅, MCP server partial (community wrappers), extensions + slash
  commands ✅, `GEMINI.md` ✅.
- **Open source:** Yes, Apache-2.0, Node/TypeScript. **Successor Antigravity CLI is Go and
  closed-source.**
- **Pricing (pre-retirement):** free OAuth tier (60 rpm / 1,000 rpd); paid AI Pro/Ultra for more.
- **Sources:** github.com/google-gemini/gemini-cli, developers.googleblog.com (Antigravity
  transition), GitHub discussion #27274

### Cursor (Anysphere)
- **Models / providers:** Multi-model catalog (Claude, GPT, Gemini, Grok, Kimi, GLM, own Composer)
  routed through **Cursor's backend**. BYOK is restricted — chat models only; Tab stays on Cursor's
  models, and the full Agent loop is reportedly gated under BYOK.
- **Routing / failover:** "Auto" is a vague intelligence/cost/reliability balancer with no published
  cheapest-capable logic; cross-provider failover undocumented.
- **Subscription:** No — its own subscription or BYOK API keys; no Claude Pro / ChatGPT Plus.
- **Local LLMs:** Unofficial only (Base-URL override + HTTPS tunnel).
- **Extensibility:** MCP client ✅ (`.cursor/mcp.json`); MCP server unverified; rules/hooks/slash
  commands ✅; extensions via Open VSX.
- **Open source:** No — closed VS Code fork; the CLI (`cursor-agent`) is a closed binary.
- **Pricing:** Free; Pro $20; Pro+ $60; Ultra $200/mo; Teams $40/user (credit-pool model).
- **Sources:** cursor.com/docs/models, cursor.com/help/models-and-usage/api-keys, cursor.com/pricing

### Windsurf → Devin Desktop (Cognition)
- **Models / providers:** Multi-provider via Cognition's cloud (own SWE models + Claude, GPT/Codex,
  Gemini, Grok, DeepSeek, Kimi, GLM). BYOK is narrow — Anthropic key only.
- **Routing / failover:** "Adaptive" auto-router exists with no cost-optimization evidence;
  cross-provider failover undocumented.
- **Subscription:** No.
- **Local LLMs:** No — cloud-only.
- **Extensibility:** MCP client ✅; MCP server unverified; workflows → slash commands, rules +
  Memories; ACP support (Codex / Claude Agent / OpenCode pluggable).
- **Open source:** No — proprietary. Editor is Electron/Monaco; the "Devin Local" *agent* was
  rewritten in Rust (this is the agent runtime, not local inference).
- **Pricing:** Free; Pro $20; Max $200/mo; Teams $80 base + $40/seat.
- **Sources:** devin.ai/blog/windsurf-is-now-devin-desktop, docs.devin.ai/desktop/models,
  devin.ai/pricing

### GitHub Copilot CLI & Copilot coding agent
- **Models / providers:** Multi-provider within GitHub's hosted catalog (Anthropic, OpenAI, Google
  Gemini, Microsoft MAI-Code). BYOK: yes for the **CLI** (OpenAI/Azure/Anthropic/any
  OpenAI-compatible incl. Ollama/vLLM/LM Studio); **no BYOK for the coding agent**.
- **Routing / failover:** **Partial — the closest of any to real auto-routing.** Documented automatic
  complexity + model-health routing (simple tasks → cheaper models) with cross-vendor failover, but
  only inside GitHub's catalog (10% discount in auto mode). None if you pin a model.
- **Subscription:** No — needs a Copilot subscription; BYOK takes API keys, not consumer plans (the
  CLI in BYOK mode needs no GitHub login at all).
- **Local LLMs:** Yes via CLI BYOK; not the coding agent.
- **Extensibility:** MCP client ✅ (GitHub MCP pre-configured); custom slash commands + agent profiles
  (CLI). MCP server not advertised.
- **Open source:** No — `github/copilot-cli` license bans modification/derivatives; the coding agent
  is a hosted service. Node/npm-distributed.
- **Pricing:** Usage-based via GitHub AI Credits (since June 1, 2026). Free $0; Pro $10; Pro+ $39;
  Business $19/user; Enterprise $39/user.
- **Sources:** docs.github.com/copilot (auto-model-selection, supported-models, byok), github.blog
  (usage-based billing, CLI GA), docs.ollama.com/integrations/copilot-cli

### Aider
- **Models / providers:** Many (15+) via **LiteLLM** (OpenAI, Anthropic, Gemini, DeepSeek, Groq, xAI,
  Azure, Bedrock, Vertex, Ollama, OpenRouter, …).
- **Routing / failover:** No automatic routing — the architect/editor model pairing is *manual*
  config; no failover.
- **Subscription:** No — API keys required.
- **Local LLMs:** Yes.
- **Extensibility:** **No native MCP** (confirmed by open issue #4506); MCP only via the separate
  third-party AiderDesk GUI. Extensible via in-chat slash commands + config.
- **Open source:** **Yes, Apache-2.0, Python** (pip).
- **Pricing:** Free, BYO-key; no first-party hosted offering.
- **Sources:** github.com/Aider-AI/aider (LICENSE.txt, issue #4506), aider.chat/docs/llms.html

### Cline (VS Code extension)
- **Models / providers:** Many — Anthropic, OpenAI, Gemini, OpenRouter (200+), Bedrock, Azure, Vertex,
  Cerebras, Groq, Ollama, LM Studio, any OpenAI-compatible.
- **Routing / failover:** No automatic routing (manual, including a different model for Plan vs Act);
  no native cross-provider failover.
- **Subscription:** Grey/uncertain — Cline added a "Claude Code" provider wrapping the local `claude`
  binary to ride a Claude Max subscription, but **Anthropic's Jan 2026 ToS enforcement** blocked
  third-party consumer-credential use; whether the wrapper still works is genuinely uncertain. ChatGPT
  Plus: no.
- **Local LLMs:** Yes (Ollama / LM Studio).
- **Extensibility:** MCP client ✅ + MCP marketplace + Skills/Plugins; custom workflows/rules; SDK +
  CLI. MCP server: no (client only).
- **Open source:** Yes, Apache-2.0, TypeScript.
- **Pricing:** Free + BYO-key; optional hosted pay-as-you-go credits; Enterprise/Teams tiers.
- **Sources:** github.com/cline/cline, cline.bot/pricing, cline.bot/blog

> **Roo Code** (a Cline fork) was **discontinued May 15, 2026** (archived, read-only). Its final state:
> multi-provider, MCP client + marketplace, Custom Modes (per-mode model assignment, manual not auto),
> Apache-2.0 / TypeScript. The community fork **ZooCode** continues it. Treat Roo Code as historical.

### opencode (SST / Anomaly)
- **Models / providers:** Very broad — **75+ providers** via the AI SDK + Models.dev, plus local
  models.
- **Routing / failover:** No automatic routing (manual `/models`); no failover.
- **Subscription:** Claude Pro/Max OAuth is **prohibited by Anthropic and unbundled as of v1.3.0**
  (community plugins only, at ToS risk); ChatGPT Plus auth reported but low confidence.
- **Local LLMs:** Yes.
- **Extensibility:** MCP client ✅ (local + remote); MCP server no; plugins ✅ (JS/TS); custom commands
  + custom agents ✅; LSP integration.
- **Open source:** **Yes, MIT.** Runtime: TypeScript on Bun (the TUI was originally Go/Bubble Tea,
  migrated to OpenTUI — TS over a native Zig core).
- **Pricing:** Free, BYO-key; optional paid "opencode Zen" gateway (pay-per-request) + a low-cost
  "opencode Go" plan.
- **Sources:** opencode.ai/docs (models, providers, mcp-servers, plugins, zen), github.com/sst/opencode

---

## Summary matrix

| Tool | Provider lock-in | Auto cost routing | Use existing sub | Cross-provider failover | Local LLMs | MCP client | MCP server | Open source | Runtime | Pricing |
|---|---|---|---|---|---|---|---|---|---|---|
| **Forge** | **No (17+ providers)** | **Yes (benchmark-ranked)** | **Yes (Claude/Codex/Gemini)** | **Yes (full catalog)** | Yes | Yes | Yes | **Yes (MIT)** | **Rust** | Free / BYO-key |
| Claude Code | Claude only (multi-host) | No | Yes (Pro/Max) | No | Proxy only | Yes | Yes | No (proprietary) | Node/TS | $20–200/mo + API |
| Codex CLI | No (OpenAI-compat) | No | Yes (ChatGPT) | No | Yes (built-in) | Yes | Yes | Yes (Apache-2.0) | Rust | Free–$100+/mo + API |
| Gemini CLI ⚠retired | Gemini only | No | Was yes (until 6/18/26) | No (same-vendor) | No | Yes | Partial | Yes (Apache-2.0) | Node/TS | Free tier (ended) / API |
| Cursor | Multi (own backend) | No (Auto≠cost) | No | Unverified | Unofficial | Yes | Unverified | No | Closed binary | $20–200/mo |
| Windsurf / Devin Desktop | Multi (own cloud) | No (Adaptive≠cost) | No | Unverified | No (cloud-only) | Yes | Unverified | No | Electron + Rust agent | $20–200/mo |
| Copilot CLI / coding agent | Multi (own catalog) | Partial (auto, in-catalog) | No | Yes (auto mode only) | Yes (CLI BYOK) | Yes | No | No | Node | Usage-based + $10–39/user |
| Aider | No (LiteLLM) | No | No | No | Yes | No (issue #4506) | No | Yes (Apache-2.0) | Python | Free, BYO-key |
| Cline | No | No | Grey (ToS-blocked) | No | Yes | Yes | No | Yes (Apache-2.0) | TypeScript | Free + optional credits |
| Roo Code ⚠discontinued | No | No | Was grey (blocked) | No | Yes | Yes | No | Yes (Apache-2.0) | TypeScript | Free (dead) |
| opencode | No (75+) | No | Prohibited (Anthropic) | No | Yes | Yes | No | Yes (MIT) | TS/Bun | Free + optional Zen |

---

## Honest caveats

- **"Auto" routers are not cost-mesh routing.** Copilot's auto mode is the closest thing to Forge's
  routing, and it is genuinely good — but it is confined to GitHub's hosted catalog and is not
  open/multi-provider. Cursor and Windsurf "auto/adaptive" modes have no published cost logic.
- Several cells above are marked **unverified** (Cursor/Windsurf cross-provider failover, MCP server
  support) because the vendors don't document them — they are not asserted as ✅ or ❌.
- Consumer-subscription bridging (riding a Claude Max / ChatGPT Plus plan from a third-party tool) is a
  moving legal target; Anthropic's 2026 ToS enforcement changed what works. Forge's first-party bridges
  to Claude Code / Codex / Antigravity use each vendor's own CLI, which is the supported path.
- Forge's own routing/failover/conservation claims are verifiable in this repo
  (`crates/forge-mesh`), and its harness-reliability claims are test-pinned — see
  [Why Forge is a better harness](harness/why-forge-is-a-better-harness.md) and
  [benchmark results](benchmarks/results.md).
</content>
