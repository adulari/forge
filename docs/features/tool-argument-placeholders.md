# Placeholder arguments

A model asked to call a tool with many optional properties often sends all of them, each with the
value that means "not given": `commands: []`, `paths: []`, `path: ""`, `cwd: ""`,
`file_pattern: ""`, `timeout_secs: 0`, `null`. A tool that tells absent from present with
`args.get(key).is_some()` then sees a contradiction.

The evidence is the first Codex comparison (gpt-6-luna on the ChatGPT login, 26 tasks, 2 seeds):
117 `shell` calls in 52 native-loop runs were rejected with ``pass either `command` or `commands`,
not both`` (157 over every native-loop run of the lane), while the 334 `shell` calls of the Codex
bridge, which reaches the same tool schema through `forge mcp-serve`, never carried `commands: []`
(106 of them used `commands` for real). The same
shape cost `read_file` (`path` with `paths`) eight calls, and `multi_edit` an empty per-edit `path`.
Forge's `stuck: shell failed 3 times (Schema)` guard fired on it, and three lost tasks (b01, b03,
d01) had rejected `shell` calls in the transcript.

## Two layers

1. **On the wire.** `codex-oauth` builds its own Responses API request, and a function tool without
   a `strict` field is `strict: true` there, the mode that asks for every property to be present.
   Forge now sends `"strict": false`, as the Codex CLI does for its tools and as the other OpenAI
   paths in Forge already did (`crates/forge-provider/src/oauth_responses.rs`).
2. **At dispatch.** Whatever the model sends, `forge_tools::normalize_args` removes the
   placeholders using the tool's own schema before the call reaches the permission rules, the hooks,
   the loop guards and the tool (`crates/forge-tools/src/arg_shape.rs`). It runs where a call is
   resolved on the direct path (`Session::resolve_model_call`, which also covers `tool_call`
   wrappers and `run_code` inner calls), for subagent children, and in `forge mcp-serve`. External
   MCP tools keep their arguments as sent: their schema is the server's, and an empty string there
   can be a real value. The transcript keeps what the model sent, because the provider's
   request-reuse compares the replayed items with what it produced.

## What counts as a placeholder

Only an **optional** property (one the schema does not list in `required`) is touched, and only
when its value is the empty value of its declared type:

| schema type | dropped when | kept |
|---|---|---|
| any | `null` | |
| `string` | `""` | `"minLength": 0` marks the empty string as a real value |
| `array` | `[]` | |
| `object` | `{}` | |
| `integer` / `number` | below the schema's `minimum` (`timeout_secs: 0` with `"minimum": 1`) | `0` where the schema allows it (`context`, a coordinate) |
| `boolean` | never | `false` is an answer |

Nested objects, including the objects inside arrays (`multi_edit.edits[]`, `update_tasks.tasks[]`),
are normalised against their own `properties`.

## Adding or changing a tool

- Give every optional property a `type`. A property with none cannot be normalised; the sweep test
  (`arg_shape::tests::every_registered_tool_loses_the_placeholders_for_all_its_optional_properties`)
  fails on it.
- Give a count that cannot be 0 `"minimum": 1`. Without it a model's `0` reaches the tool as a
  value (`head_limit: 0` once meant "return nothing").
- If an empty string is a legitimate value of an optional property, say `"minLength": 0`.
- Do not rely on the normaliser alone for an either/or pair. `shell` and `read_file` also read a
  blank `command`, a blank `path` and an empty list as absent themselves, so a caller that does not
  go through the dispatch path (a script, a test) gets the same answer.

## The permission rules see the same call

`forge_types::shell_command_text` is what permission rules, auto-mode risk and the other
shell-aware checks read. A blank `command` next to a non-empty `commands` is a placeholder, so the
batch is what gets judged. Reading the blank string instead would let the batch run unjudged once
the tool stopped rejecting the pair.

## `apply_patch` hunks in the Codex envelope

The same models send Codex-style patches, where a chunk with no `-`/`+` line (context only) or a
`@@ <line>` header is a position: Codex applies the next chunk after it. Forge treated a
context-only chunk as an edit whose `old` and `new` were identical and rejected the whole patch.
That was 15 of the 19 failed `apply_patch` calls in the 52 native-loop runs, and the b01 retry
loop; 3 more were a `-`/`+` pair restating the same text, which stays an error.

Such a chunk is now not a hunk. It, or the `@@` header text, scopes the next hunk to the first
match after those lines, so a hunk whose `old` text occurs many times (a closing `}`) lands where
the patch meant. A scope that is not in the file, or an `old` that does not follow it, is ignored:
the scope only chooses between matches and never makes an edit fail that applied without it. A
patch that is nothing but context is refused with that reason.
