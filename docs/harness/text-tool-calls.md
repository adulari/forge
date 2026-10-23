# Tool calls written as text

A model is meant to call a tool through the provider's function-calling interface. Sometimes it
writes the call into its reply instead, as markup, and nothing runs. Forge recovers the formats
models actually produce, refuses the ones it cannot read safely, and tells the model to try again.

The code is `crates/forge-provider/src/tool_recovery.rs` (the plain formats),
`crates/forge-provider/src/dsml.rs` (DeepSeek) and the guard in
`crates/forge-core/src/model_loop.rs`.

## What is recovered

| Format | Looks like |
| --- | --- |
| Anthropic style | `<invoke name="T"><parameter name="p">v</parameter></invoke>` |
| Qwen / Ollama | `<tool_call>{"name":"T","arguments":{...}}</tool_call>` |
| Llama / Groq | `<function=T>{"p":"v"}</function>` |
| DeepSeek DSML | see below |
| Bare JSON | a reply that is only `{"name":"T","arguments":{...}}` |

The recovered calls run exactly like structured ones, under the same permission rules. The markup
is removed from the answer the user sees.

## DeepSeek DSML

DeepSeek's chat template writes tool calls between `｜DSML｜` tags (the bars are U+FF5C). The
serving stack normally turns them into a structured call. When it does not, they arrive as
plain text. Three dialects exist, and the parser reads all of them:

```
V3.2   <｜DSML｜function_calls> <｜DSML｜invoke name="T"> <｜DSML｜parameter name="p" string="true">v</｜DSML｜parameter> ...
V4     <｜DSML｜tool_calls>     same invoke and parameter tags
V4.1   <｜DSML｜ calls>          a space after the namespace on every tag, wrapper shortened to `calls`
```

Models also double the bars (`｜｜DSML｜｜`), and a gateway may flatten them to ASCII `|`; all of
these parse. A parameter with `string="true"` is literal text and is kept byte for byte.
`string="false"` is a JSON value (number, boolean, array, object). A block may hold several
`invoke` elements; they run in order.

### What counts as malformed

The parser is relaxed about spelling and strict about structure. A reply runs none of its
DSML calls when any of these is true:

- a tag is cut off, or an `invoke` or `parameter` is never closed
- the wrapper is opened and never closed (the reply may have been truncated mid-block)
- a `string="false"` value is not valid JSON, or `string` is neither `true` nor `false`
- an `invoke` has no name, a parameter has no name, or a parameter is given twice
- there is text, or a tag of the wrong kind, inside the block

Running the complete calls of a half-finished block would execute something the model never
finished asking for, so nothing runs. The block is also cut from the visible answer. A DSML example
inside a Markdown code fence is documentation, not a call, and is left alone.

## When nothing could be recovered

If the reply still looks like an unexecuted call, the loop nudges the model, at most twice, to
make the call through the function-calling interface. If it keeps failing, the turn ends with a
warning (`model kept emitting tool calls as text that never executed`) and the answer shown has the
markup removed.

## Why it happens without tools

Forge does not send tools with a trivial-tier turn unless the prompt shows a workspace intent
(`crates/forge-core/src/tool_intent.rs`). With no `tools` on the request, the serving stack has
nothing to parse a call against, and the model, which has no definitions to follow, improvises
(the V4.1 shape above, with a space and `calls`, is what `deepseek-v4.1-flash` produced). Two
things close the gap:

- a prompt that names a source file (`pkg/limits.py`, `src/lib.rs:12`, `Cargo.toml`) is shown tools
- a model that writes a call as text, or whose call was recovered from text, gets the tools on its
  next request and for the rest of the session

## Live stream

`dsml_stream_guard.rs` keeps DSML out of the text deltas while they stream. Anything that could
still become a `<｜DSML｜` opener is held back until it is clear it is not one; once an opener is
confirmed the rest of the reply is withheld.
