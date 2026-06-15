# Free cloud models in the Mesh

Forge routes any `provider::model` id through the [Model Mesh](../roadmap.md) cost-aware router
(FR-5): it picks the **cheapest usable** candidate per tier, and any model **not** listed in the
pricing table costs **$0** — so genuinely-free providers win automatically when their key is set,
and the mesh falls back down the candidate list otherwise.

These all work through Forge's `genai` backend; most are **native genai adapters** (just set the
key), and **Cerebras** is wired via a custom OpenAI-compatible endpoint resolver.

## Providers & keys

| Forge namespace | Free? | API-key env (`forge auth <name>`) | Notes |
|---|---|---|---|
| `groq::` | free tier | `GROQ_API_KEY` | Fast. `llama-3.3-70b-versatile`, `llama-3.1-8b-instant`, `qwen3-32b` — tool-calling supported. |
| `gemini::` | free tier | `GEMINI_API_KEY` | Free tier = **Flash family** (`gemini-2.5-flash`, `gemini-3-flash`); Pro left the free tier in 2026. Tools supported. |
| `open_router::` | `:free` models | `OPEN_ROUTER_API_KEY` | Append `:free` to a model id (e.g. `open_router::deepseek/deepseek-r1:free`). Rate-limited. Tool support varies by model. |
| `opencode_go::` | free (OpenCode Zen) | `OPENCODE_GO_API_KEY` | OpenCode Zen's curated free coding models (designed for tool calling). |
| `github_copilot::` | free tier | `GITHUB_TOKEN` | GitHub Models inference gateway (`github_copilot::openai/gpt-4.1-mini`, …). |
| `mimo::` | free tier | `MIMO_API_KEY` | Xiaomi MiMo. |
| `minimax::` | free tier | `MINIMAX_API_KEY` | MiniMax. |
| `cerebras::` | free tier | `CEREBRAS_API_KEY` | **No native genai adapter** — Forge retargets the OpenAI-compatible `api.cerebras.ai` endpoint via a service-target resolver. |

> **Model ids change over time** and free tiers shift month-to-month — treat the shipped defaults
> and the ids above as a starting point and edit `[mesh.models]` to taste. **Tool/function-calling
> support varies per free model**; route tool-heavy tiers to models documented to support tools
> (Groq llama-3.3-70b, Gemini Flash, OpenCode Zen coding models).

## Default tiers (shipped)

Each tier leads with a free candidate, then falls back:

```toml
[mesh.models]
trivial  = ["groq::llama-3.1-8b-instant", "ollama::llama3.2"]
standard = ["groq::llama-3.3-70b-versatile", "gemini::gemini-2.5-flash", "openai::gpt-4o-mini"]
complex  = ["groq::llama-3.3-70b-versatile", "claude-cli::", "anthropic::claude-opus-4-8"]
```

A free model with a configured key (cost $0) wins the cost-aware pick; with no key it's skipped and
the mesh routes to the next usable candidate (local `ollama::`, a subscription bridge, or a metered
API model). Set keys with `forge auth groq` (etc.) or the provider's env var.

## Example: an all-free setup

```toml
[mesh.models]
trivial  = ["groq::llama-3.1-8b-instant", "ollama::llama3.2"]
standard = ["opencode_go::deepseek-v4-flash", "groq::llama-3.3-70b-versatile"]
complex  = ["cerebras::llama-3.3-70b", "open_router::deepseek/deepseek-r1:free"]
```
