# Feature: image delivery to models

An image attached to a turn (paste, `/image`, remote upload) is only useful if the model that
answers can actually see it. This page covers the provider side: how each transport carries the
image, how Forge decides a model can read it, how routing follows the image, and what happens when
the chosen model cannot.

## Transports

| Provider | How the image travels |
|---|---|
| OpenAI, Anthropic, Gemini, Bedrock, Ollama, OpenAI-compatible custom providers (InferHub, ModelGateway, ...) | genai `ContentPart::Binary` on the user message |
| `codex-oauth`, `xai-oauth` (Responses API) | `input_image` parts (`data:<mime>;base64,...`) next to an `input_text` part (`crates/forge-provider/src/oauth_responses.rs`) |
| `claude-cli` bridge | stream-json user message with `image` content blocks. One-shot turns switch stdin to `--input-format stream-json` only when an image is present; the persistent transport already speaks it (`crates/forge-provider/src/cli_provider/images.rs`) |
| `codex-cli` bridge | decoded to temp files and passed as `codex exec --image <file>`; the files are deleted when the turn ends |
| `agy-cli` | text only (`agy -p <prompt>`): reports no vision regardless of model |

One image turn budget applies everywhere: only the **latest user turn that has images** goes on the
wire (`forge_provider::last_image_turn`). A CLI bridge session that already saw an image (`--resume`
or the persistent process) is not sent it again.

## Does this model read images?

`forge_mesh::catalog::{supports_vision, vision_capability}` resolve in this order:

1. the provider has no image channel (`agy-cli`) -> no;
2. catalog data -> OpenRouter `architecture.input_modalities` and the first-party providers in
   models.dev (`modalities.input`), fetched in the same sweep as context windows and prices and
   cached in `<data dir>/vision-modalities.json`. Gateway ids (`inferhub::glm-5.3-flash`) borrow the
   same model's row by basename. models.dev aggregators are ignored because they disagree with each
   other about the same model;
3. the name heuristic (`crates/forge-mesh/src/vision.rs`), which now also covers Kimi K2.5+, GLM-V,
   DeepSeek-VL, Mistral Small/Medium/Large 3+, Qwen-VL, Gemma 3+, LLaVA and other vision families.

`vision_capability` returns `None` for a model nothing describes; such a model still receives the
image and the provider's own answer stands.

## Routing follows the image

Image-awareness is "the transcript still carries an image that will be sent", not just "an image is
pending for this turn". A follow-up ("and what colour is the button?") or a tool step after the
image turn therefore routes only to vision-capable models, and so does failover.

## `[mesh] vision_fallback`

What happens when the selected model (routed or pinned) is known to be text-only:

| Value | Behaviour |
|---|---|
| `warn` (default) | The request goes out text-only with an `[image attachment omitted ...]` marker, and a warning is shown once per model and image: `<model> can't read images; sent text only - set mesh.vision_fallback = "describe" or "switch"`. |
| `describe` | A vision-capable side call (never a subscription, honours zero-retention and the disabled list, tried on up to three candidates) writes a detailed description including visible text verbatim. It replaces the image in the request and is cached per image, so a tool loop or follow-ups pay for it once. If no vision model is available it degrades to `warn`. |
| `switch` | The turn is routed to the best vision-capable model even when a model is pinned (a one-turn exception to the pin), with a visible note. Failover hops skip text-only models. If no vision model is usable it degrades to `warn`. |

Config only; there is no per-session override.

## Not covered

Images returned by tools (MCP screenshots, browser and device captures) still reach the model as a
text marker only; carrying them needs a tool-result image channel through the tool, transcript and
provider layers.
