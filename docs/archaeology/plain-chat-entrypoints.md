# Code archaeology: plain chat and natural-language entry points

## Summary

The CLI root mixed two non-TUI entry lifecycles into the animated chat owner. Plain chat now owns line-oriented session hooks and prompt execution; natural-language mode owns one shell-oriented query. Both continue to use the canonical session builder, permission broker, routing, persistence, and turn pipeline.

## History and invariants

- `85a2bb51` introduced multi-turn plain chat.
- `09eee8e9` introduced `forge nl` as a natural-language shell interpreter.
- `36981e13` added UserPromptSubmit and SessionStart/SessionEnd hook events.
- `b8e30871` rooted hooks in each session workspace.
- `2a4fbb7c` moved these entry points from the original CLI god file into `cli/commands/run.rs`.

TTY chat still selects the animated TUI unless `--plain` is requested. Plain chat runs SessionStart before the input loop and SessionEnd after normal EOF or `/quit`; a turn error returns early, as before. A blocked prompt skips only that turn. Resume ids, provider pins, permission mode, and workspace rooting flow through the shared session builder.

## Boundaries

`plain_chat.rs` owns TUI/plain selection and the line-mode lifecycle. `natural_language.rs` owns fixed-argv environment discovery and one fresh NL turn. `chat_action` remains dispatch policy, while animated input/event handling remains in `run_chat_tui`.

Natural-language mode does not execute an assembled shell command: its fixed local `git` probes use argv directly, and model-requested commands still pass through the shell tool and permission broker. Repository branch/commit text is untrusted context and is therefore delimited as data rather than instructions. NL now applies the same prompt and session hook policy as other prompt-carrying entry points.

## Interface as test surface

The compiler and CLI dispatch wiring exercise ownership of `chat` and `nl_cmd`; shared session, routing, hook, and turn tests cover their delegated boundaries. No end-to-end test currently invokes either interactive entry point directly. Pure context formatting is characterized independently.

## Leave alone

- A blocked prompt skips only that turn.
- SessionEnd hooks run after normal EOF or `/quit`.
- Plain mode may block for update checking; TUI startup may not.
- Natural-language mode never bypasses the shell permission broker.
