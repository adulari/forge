# Tool vertical slice — workspace confinement phase

This mechanical extraction isolates the session-workspace tool adapter into
private `workspace.rs`.

## Ownership

The module owns workspace argument rooting, containment validation,
`SESSION_WORKSPACE` scoping, and the adapter that wraps registered tools.
`ToolRegistry` remains the public composition and registry owner. No public
Tool trait, schema, tool name, permission path, sandbox policy, or workspace
behavior changes.

## Result

| Measure | Before | After |
|---|---:|---:|
| `forge-tools/src/lib.rs` implementation lines | 286 | 170 |
| `workspace.rs` implementation lines | — | 133 |
| Workspace implementation files ≤500 | 121/186 | 122/187 |
| Workspace implementation files ≤800 | 151/186 | 152/187 |

## Verification

- warnings-denied Tools Clippy across targets/features
- Tools suite: 105 passed, 2 live-network tests ignored
- formatting and architecture guard passed

No side-effect authorization, provider request, persistence, or runtime policy
changed; no provider benchmark is applicable.
