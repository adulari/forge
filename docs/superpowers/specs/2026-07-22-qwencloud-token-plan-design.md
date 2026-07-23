# Qwen Cloud Token Plan Integration Design

## Goal

Make an individual Qwen Cloud token-plan subscription usable through Forge on this machine immediately, then make Qwen Cloud a built-in provider for all Forge users. The integration must support authenticated model discovery, ordinary and streaming completions, tool calls, explicit `qwencloud::<model>` selection, and Forge model-mesh discovery and routing.

## Scope

The work has two phases in this order:

1. Register and verify Qwen Cloud on this machine using Forge's existing runtime OpenAI-compatible provider support.
2. Add the same provider definition to Forge's built-in registry with tests and documentation.

This design does not add a Qwen-native protocol adapter, subscription management, billing or quota APIs, or a Forge-hosted credential service. A dedicated adapter is only warranted if live compatibility tests demonstrate behavior the shared OpenAI-compatible adapter cannot support.

## Architecture

Qwen Cloud uses Forge's existing custom OpenAI-compatible provider path:

- Namespace: `qwencloud`
- Base URL: `https://token-plan.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1`
- Credential environment name: `QWENCLOUD_API_KEY`
- Credential storage: Forge's OS-keyring entry for the `qwencloud` namespace
- Pricing classification: subscription-backed, not free
- Model source: authenticated `/models` discovery; the built-in row has no speculative seed models because availability is subscription-specific

The data path is:

1. Forge loads the built-in and runtime custom-provider registries.
2. Forge resolves the credential from the `QWENCLOUD_API_KEY` environment variable or the OS keyring, without writing the token into `config.toml`.
3. The shared discovery client calls the provider's `/models` endpoint with bearer authentication.
4. Returned model IDs are namespaced as `qwencloud::<provider-model-id>`.
5. The existing OpenAI-compatible client handles chat completions, streaming, and tool calls for explicitly selected models and mesh-routed requests.

## Machine-Local Setup

The first execution step registers a runtime custom provider with `forge provider add`. The supplied token is then sent only through non-echoing standard input to `forge auth qwencloud --replace`, which stores it in the OS keyring. It must not appear in command arguments, environment dumps, config files, source files, commits, or diagnostic output.

Each verification command runs in a fresh Forge process because the custom-provider registry is process-lifetime immutable after loading. Local success requires all of the following:

1. `forge provider list` reports `qwencloud` with a key set.
2. Authenticated `/models` discovery returns at least one namespaced model.
3. A short non-streaming completion succeeds.
4. A streaming completion returns incremental content and completes cleanly.
5. A forced tool-call request returns a structurally valid tool call.
6. A Forge run pinned to a discovered `qwencloud::<model>` succeeds.
7. The discovered models are visible to model-mesh selection and a routed request succeeds where the existing selection policy permits a subscription-backed model.

After the built-in provider is installed, the temporary `[[providers.custom]]` entry is removed. The keyring entry remains valid because both configurations use the same namespace.

## Built-In Integration

The built-in change adds one data row to `CUSTOM_OPENAI_PROVIDERS`. It reuses all existing custom-provider behavior rather than branching provider-specific request code. Documentation lists the token-plan endpoint, `forge auth qwencloud`, model discovery behavior, and namespaced model selection.

The provider remains marked `free: false`. A prepaid or subscription token plan is not a standing free tier, and classifying it as free could route workload unexpectedly.

## Error Handling and Safety

- Registration validates and normalizes the HTTPS base URL through the existing custom-provider validation.
- Missing credentials produce the existing provider-key diagnostic and never fall back to another provider under an explicitly pinned `qwencloud::` model.
- HTTP 401 or 403 is reported as an authentication or entitlement failure without logging the bearer token.
- `/models` failure is reported independently from completion compatibility. No unverified model IDs are baked in as fallback seeds.
- Streaming protocol errors and malformed tool-call payloads remain provider-call failures with the existing retry and diagnostic behavior.
- The temporary runtime registration is removable without touching unrelated provider configuration. Key removal is a separate explicit operation.
- The supplied credential should be rotated after verification because it was pasted into chat.

## Testing

Automated tests cover:

- The built-in registry row's namespace, normalized endpoint, environment name, paid classification, and empty seed list.
- Provider lookup and authentication recognition for `qwencloud`.
- OpenAI-compatible resolver construction and bearer-key injection without exposing key contents.
- Authenticated model discovery and `qwencloud::` namespacing using a mock server.
- Non-streaming, streaming, and tool-call response handling through the shared adapter where existing coverage does not already prove it generically.
- Runtime/built-in namespace collision behavior so the built-in wins while the temporary runtime entry remains safely removable.
- CLI/provider documentation examples.

Live verification uses the user's token-plan subscription and the seven machine-local acceptance checks above. Tests and captured output must redact credentials.

## Completion Criteria

The task is complete when the machine-local acceptance checks pass, the built-in provider tests and relevant workspace checks pass, documentation is updated, the temporary runtime registration is safely removed after installing the built-in-enabled binary, and no credential has been persisted outside the OS keyring or exposed by tooling.
