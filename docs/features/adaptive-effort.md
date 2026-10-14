# Adaptive reasoning effort

`[mesh] adaptive_effort` (default `true`) runs the model at low reasoning effort for the step after
a tool round in which every call was provably read-only and succeeded. Reading the next file or
re-running a search rarely needs deep reasoning, and output tokens cost 5-8x input. The configured
effort comes back after:

- a tool call that failed or was denied,
- a write (any call that is not provably read-only, including a mutating shell command),
- a user turn or a steer,
- a step with no tool calls.

"Provably read-only" is the same test the concurrent read-only batch uses (`is_concurrent_readonly`):
read-side tools, and shell lines the broker would allow outright as read-only.

## When it does nothing

- **The provider is not listed** in `[mesh] adaptive_effort_providers`.
- **The previous response reported no reasoning tokens** (`usage.reasoning_tokens` absent or 0):
  the model either does not reason or cannot show a saving, so there is nothing to cut or measure.
- **`/effort exact <level>`** is an instruction and is never lowered. A plain `/effort <level>`
  pin or the mesh's measured rung is the configured effort that gets lowered and restored.
- **The configured effort is already `low`.**

## Why per provider: the prompt cache

Lowering effort only saves money when the request still hits the provider's prompt cache. Each
default was checked against the adapter that builds the request:

| Provider | Where the effort goes | Cache effect | Default |
|---|---|---|---|
| `openai` | top-level `reasoning_effort` (genai OpenAI adapter) | OpenAI caches the prompt prefix (messages, tools, images, schema); the field is outside it | on |
| `codex-oauth` | `reasoning.effort` beside `input` (codex_oauth.rs) | same OpenAI cache | on |
| `gemini` | `generationConfig.thinkingConfig` (genai Gemini adapter) | Gemini caches `contents`/`systemInstruction`/`tools`; explicit cachedContent holds only those | on |
| `anthropic`, `bedrock` | `output_config.effort` or `thinking.budget_tokens` | Anthropic documents that a `thinking` or `effort` change invalidates the messages cache | off |
| `openrouter` | `reasoning`, forwarded to the upstream | the upstream may be Anthropic | off |
| open-weight hosts (`groq`, `cerebras`, `nvidia`, `together`, `fireworks`, `deepseek`, `ollama`, ...) | `reasoning_effort` or a thinking flag | the chat template renders the level into the prompt (gpt-oss harmony writes `Reasoning: <level>` into the system message), so the prefix changes from the first token | off |
| CLI bridges | a process argument | the persistent claude bridge would have to restart | off |

The critic's claim behind this feature, that the effort parameter sits outside the cached prefix
on both OpenAI and Anthropic, holds for OpenAI and not for Anthropic.

To opt another provider in after measuring it, list it:

```toml
[mesh]
adaptive_effort_providers = ["openai", "codex-oauth", "gemini", "xai-oauth"]
```

Set `adaptive_effort = false` to turn it off everywhere.

## Measuring it

Every lowered request is logged at debug level (`adaptive effort: low after a read-only round`).
The saving shows in the `usage.reasoning_tokens` column (migration 42): compare reasoning tokens
per call, and cost per solve in `forge bench report`, between runs with the switch on and off. The
cache instrument (`forge stats cache`) shows whether hits held. The plan gates keeping it on by
default on cost per solve.

Code: `crates/forge-core/src/adaptive_effort.rs` (state and decision),
`crates/forge-config/src/adaptive_effort.rs` (the provider list and why).
