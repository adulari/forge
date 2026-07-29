# Code archaeology: OpenAI wire translation

## Boundary

`api_serve/wire.rs` owns the translation between the OpenAI-compatible request shape and Forge's
domain: request/`response_format`/tool deserialization, message and tool-call conversion, the
prompt cache key, model-pin resolution, and the text the mesh classifies on. `api_serve.rs` keeps
the server itself — routing, auth, handlers, streaming, and response assembly.

## Why the seam is here

The server's value proposition is that an *existing* client works unchanged, so this module is
where the shapes real clients send are absorbed: content that is either a string or a content-part
array, tool calls whose arguments arrive as stringified JSON, `response_format` in two spellings,
and `model` as `auto` / `mesh` / a concrete id. Keeping that tolerance in one owner stops it
leaking into the handlers as scattered special cases.

## Routing text is part of the translation

`routing_prompt` derives what the mesh classifies from the conversation, bounded and — for
machine-generated payloads — reduced to structure. That belongs with the wire layer for the same
reason: it exists because of what clients send (one enormous pasted document must not dominate
routing), not because of how the server runs.

## Interface

Everything is `pub(super)`; the handlers import it with a module-local glob, so no call site
changed and nothing new is exposed outside `api_serve`.

## Characterization

The existing API tests — content-part flattening, tool-call parsing, pin resolution, cache-key
stability, routing-text bounding — continue to exercise this code through the handlers and pass
unchanged.
