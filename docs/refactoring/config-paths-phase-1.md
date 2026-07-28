# Config filesystem policy extraction

This architecture-only phase moves platform-location and project-initialization
policy out of the Config root into private `paths.rs`.

## Ownership and compatibility

`paths.rs` owns XDG/platform config and data directories, external CLI home
locations, project guidance discovery, and the opt-in auto-setup marker. All
functions and `ProjectInitialization` remain deliberate root re-exports, so
callers retain their existing `forge_config::*` paths.

No configuration precedence, file format, secret lookup, user/project path,
auto-initialization marker, or filesystem result changes.

## Result

| Measure | Before | After |
|---|---:|---:|
| Config root implementation lines | 4,344 | 4,252 |
| `paths.rs` implementation lines | — | 104 |
| Workspace implementation files ≤500 | 122/187 | 123/188 |
| Workspace implementation files ≤800 | 152/187 | 153/188 |

## Verification

- warnings-denied Config Clippy across targets/features
- Config suite: 122 passed
- formatting and architecture guard passed
