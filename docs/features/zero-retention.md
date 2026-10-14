# Zero-data-retention routing

`[privacy] zero_retention = true` restricts Forge to providers known not to retain your prompts.
It is off by default and costs no tokens; what it costs is choice, since most hosted providers are
excluded until you vouch for them.

```toml
[privacy]
zero_retention = true

# Optional. `false` vouches that a provider does not retain prompts under YOUR account terms
# (for example an enterprise zero-retention agreement); `true` marks one as retaining.
[privacy.retains]
openrouter = false
```

## What counts as zero-retention

Forge does not guess. A provider is allowed only when:

1. you list it as `false` under `[privacy.retains]`, or
2. it is in the built-in table (`RETENTION_TABLE`, `crates/forge-mesh/src/privacy.rs`) as not
   retaining.

**Every other provider is treated as retaining**, including every hosted API and both CLI bridges.
Their retention terms depend on the account, plan and region, none of which Forge can see. A
user entry wins over the table in both directions.

The built-in table currently has one entry:

| Provider | Retains | Evidence |
|---|---|---|
| `ollama` | no | Forge calls `OLLAMA_HOST` (default `localhost:11434`), so prompts stay on this machine. Treated as retaining when `OLLAMA_HOST` points at a non-loopback host, and for Ollama cloud models (a `:cloud` or `-cloud` tag), which the local server forwards to ollama.com. |

Add an entry only with evidence from how Forge talks to the provider. Anything else belongs in the
user's `[privacy.retains]`, not in the shipped table.

## Where it is enforced

- **Routing.** `HeuristicRouter::is_usable` drops retaining models from every tier and from the
  failover chain. `decide_with_pin` then runs `enforce_on_decision`: an unpinned pick that is
  still retaining moves to its first allowed fallback, and a retaining pin or an empty chain becomes
  an `unroutable` decision. The turn fails with that rationale before any request is sent.
- **Session pins.** An in-session `/model` pin is applied after routing, so core runs
  `enforce_on_decision` again after its pin backstop.
- **Side calls.** Recap, suggestion, memory capture, shell diagnosis and compaction route through
  the same router. Under a pin, `side_call_model` also filters its trivial-tier candidates.
- **Every request.** `Session::build` wraps the provider in a guard
  (`crates/forge-core/src/zero_retention.rs`) that refuses any retaining model with a
  `ProviderError::Request` naming the reason. That covers the paths that pick a model without
  the router: the architect planner, the failover last resort, and `forge api` exact-model
  requests. A refusal is a policy decision about this session, so it never benches the model in
  the shared health store.
- **Calls outside a session.** The tier classifier (`[mesh] classifier = "llm"`/`"hybrid"`, which
  sees every prompt), the CLI bridge's auto-mode command classifier in `mcp-serve`, `forge skill`
  distillation (which sends a stored transcript) and `forge assay` (which sends source) all use a
  guarded provider too. The classifier candidate list drops retaining models up front, so with
  none left Forge classifies heuristically instead of sending the prompt to, say, Groq.
- **OpenRouter.** Every request the guard lets through carries `deny_data_collection`. On
  OpenRouter that becomes `provider: {"data_collection": "deny"}` in the request body, which
  restricts routing to upstream endpoints that do not collect prompts. OpenRouter is only reached
  at all if you vouched for it in `[privacy.retains]`; the field is a second line of defense,
  not the reason it is allowed. Many `:free` endpoints collect data, so expect fewer of them to
  answer.

## Seeing what it dropped

`forge mesh explain "<prompt>"` prints a `zero-retention mode dropped:` section listing each
removed candidate and its reason; `--json` carries the same list as `privacy_drops` (always present,
empty when the mode is off). The `/mesh` overlay appends a one-line summary to the rationale.

## Limits

- The settings are read when a session (or `forge api`) starts; changing them mid-session applies
  to the next session.
- A provider you vouch for is trusted on your word. Forge cannot verify a provider's retention.
- With only retaining providers configured, every turn fails with an explanation. That is the
  intended behavior: the alternative is sending the prompt anyway.
