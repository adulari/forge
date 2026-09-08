# Feature: native token savings — RTK and Headroom

> Two integrations that shrink what reaches the model without changing what Forge does. Both
> detect their prerequisite and switch themselves on (`auto`); both can be forced `on`/`off`.
> Touches `forge-tools` (`shell/rtk.rs`), `forge-provider` (`headroom.rs`, the genai
> service-target resolver, the `claude` bridge env), `forge-config` (`[shell] rtk`,
> `[mesh] headroom`) and `forge-cli` (`run/setup.rs`, `run/session.rs`).

## 1. RTK — compact shell output

[RTK](https://github.com/rtk-ai/rtk) ("Rust Token Killer") runs a command and prints a compacted
rendering of its output. When `rtk` is on `PATH`, the `shell` tool prefixes eligible commands with
it, so the model reads the compact form and the raw bytes never enter the transcript.

```toml
[shell]
rtk = "auto"          # auto (default: on when rtk is on PATH) | on | off
rtk_skip = ["cargo"]  # programs to leave unfiltered
```

Measured on this repository (o200k tokens, `scripts` not involved — plain `sh -c`):

| command                              | raw  | rtk | saved | fidelity                                   |
|--------------------------------------|------|-----|-------|--------------------------------------------|
| `cargo test -p forge-agent-config --lib` | 2551 | 16  | 99%   | "144 passed (1 suite)"; failures verbatim  |
| `cargo check -p forge-agent-types`   | 95   | 30  | 68%   | compile errors preserved in full           |
| `cargo clippy -p forge-agent-types`  | 54   | 8   | 85%   |                                            |
| `ls -la crates`                      | 597  | 75  | 87%   | names + sizes; perms/mtime dropped         |
| `ls -la` (repo root)                 | 1459 | 353 | 76%   |                                            |
| `find crates/forge-tui -name '*.rs'` | 447  | 130 | 71%   | tree form                                  |
| `wc -l crates/forge-core/src/lib.rs` | 10   | 3   | 70%   |                                            |
| `git status`                         | 23   | 12  | 48%   |                                            |

What is **not** rewritten, and why — each was measured or observed to lose something an agent
needs:

- `grep` / `rg`: `rtk grep` reinterprets flags (`grep -h '^name' …` became `rtk grep -h` = help)
  and saved 0% on real searches.
- `cat`: `rtk read` filters file contents; the agent reads files to edit them.
- `git diff` / `git show`: the condensed diff drops hunk context (11.8% saved, context lost).
- `git log`: 0% saved.
- Anything the model parses structurally (`gh … --json`, `curl`, `psql`, `docker`, `kubectl`):
  not measured here, so not rewritten.

Eligible today: `ls`, `tree`, `find`, `wc`, `git status`, `cargo {build,check,test,clippy}`,
`npm {test,run}`, `pnpm {test,run,build,lint}`, `npx {tsc,eslint,vitest,jest,prettier,playwright}`,
`vitest`, `jest`, `tsc`, `eslint`, `prettier`, `playwright`, `dotnet {build,test,restore}`. Only the
first segment of a pipeline is prefixed (`cargo test 2>&1 | tail -5` → `rtk cargo test 2>&1 | tail
-5`); env assignments, `cd … &&` chains, `sudo`, multi-line scripts and anything already routed
through rtk are left alone. PTY, background and poll runs are never rewritten.

The tool result header says when it happened — `shell: exit 0 in 132ms  (rtk-filtered; \`rtk proxy
<cmd>\` for raw)` — so the model can ask for the unfiltered output when the compact form is not
enough. RTK also tees the full output of every filtered run to `~/.local/share/rtk/tee/`.

Detection probes `rtk --version` once per process and requires the banner to start with `rtk `
(the unrelated "Rust Type Kit" also installs an `rtk`). `rtk = "on"` insists on rtk even when the
probe failed, so a missing binary surfaces as the command's own error instead of silently running
unfiltered.

## 2. Headroom — a compressing proxy in front of the providers

[Headroom](https://github.com/chopratejas/headroom) is a local HTTP proxy that sits between an LLM
client and the provider and compresses what it forwards — tool results, repeated prior turns,
oversized file dumps — while keeping the provider's prefix cache warm. It speaks the OpenAI,
Anthropic and Gemini wire formats and forwards to any OpenAI-compatible upstream named in an
`x-headroom-base-url` header.

```toml
[mesh]
headroom = "off"                       # off (default) | auto (on when a healthy proxy answers) | on
headroom_url = "http://127.0.0.1:8787" # default
```

At startup Forge probes `GET /health` once (a quarter-second budget; a loopback refusal is
instant) and records the decision process-wide. When active:

- **OpenAI-wire adapters** (native `openai`, `groq`, `xai`, `deepseek`, `openrouter`, … and every
  custom OpenAI-compatible provider) are retargeted at the proxy's `/v1/chat/completions` with the
  bearer key and the real upstream in `x-headroom-base-url`.
- **Anthropic** and **Gemini** adapters only get their endpoint swapped; the proxy forwards those
  formats to their canonical hosts itself.
- The **`claude` CLI bridge** gets `ANTHROPIC_BASE_URL` pointed at the proxy (the same thing
  `headroom wrap claude` does).
- **Responses-API adapters** (OpenCode Go/Zen `muse-*`/`gpt-*`/`grok-*`) and anything whose key
  cannot be materialised as a bearer at resolve time go direct rather than half-routed.

### Does it help? What was measured

**On Forge's own direct-API traffic: no, not today.** With routing on, the proxy saw 16 Forge
requests (`meta::muse-spark-1.3-contributor`, a full coding session) totalling 595,657 input
tokens and returned 594,391 — **0.2% removed** — at an average 0.4 s of optimisation latency per
request (max 3.8 s; its Kompress model repeatedly hit its 25 ms deadline and skipped chunks). Forge
already bounds every tool result (`shell` 64 KB budget, `tool_detail` caps) and prunes bulky tool
logs at each turn boundary, so the proxy finds little left to compress. That is why the default is
`off`: the integration is there for the day a provider/model pair does compress well, one config
line away, but it is not switched on by an assumption.

**On Codex it did nothing either — and that one was a Headroom bug, now patched here.** Its own
`/stats` showed 13 Codex requests, 0 compressed, `codex_ws.units_total = 0`. Wire-capturing a
Codex 0.153 turn showed why: Codex sends only deltas (`previous_response_id`), and the only
compressible item, the tool result, arrives as `custom_tool_call_output` whose `output` is a
**list of `input_text` parts** (a status header plus the body), while Headroom's Responses
extractor only accepted a plain string — every Codex tool result was silently classed
`output_type_without_text_slot`. With the patch in
`docs/patches/headroom-0.31.0-codex-list-tool-output.patch` (applied to the local install, restart
the `headroom-init-user` service) the same turn compresses: one tool-result unit, 3,478 → 2,556
tokens, answer unchanged. Reapply after `headroom update` until it is upstream.

## 3. Tests

`forge-tools` (`shell/rtk.rs`): the rewrite rules, the deny list, the skip list, the header tag.
`forge-provider` (`headroom.rs`, `genai_provider.rs`): the health probe against a local listener,
`on`/`off` without probing, per-adapter retargeting, and the go-direct cases.
`forge-cli` (`mcp_serve.rs`): the shell-tool builder with rtk pinned off still returns `None` when
no sandbox knob is set.
