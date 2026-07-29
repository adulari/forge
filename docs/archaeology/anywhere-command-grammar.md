# Code archaeology: Anywhere command grammar

## Boundary

`cli/args/anywhere.rs` owns the Clap grammar for managed Forge Anywhere commands: enrollment, host lifecycle, encrypted handoff/share, durable jobs, device management, and logout. It is independent from Anywhere execution in `anywhere/`.

## Interface

`AnywhereCmd` is the dispatch contract consumed by `anywhere_cmd`; `ShareExpiry` is consumed by the share operation. Their value-enum spellings (`24h`, `7d`, `30d`) and defaults are stable CLI compatibility surface.

## Characterization

`anywhere_exposes_approval_and_explicit_recovery_fallback` verifies representative approval and recovery command parsing. Execution semantics remain covered by the Anywhere command and relay tests.
