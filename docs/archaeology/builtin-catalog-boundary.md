# Code archaeology: compiled-in skill/command catalog

## Boundary

`forge-skills/src/builtin.rs` owns everything Forge ships inside the binary: the compiled-in
`rust-best-practices` skill (parsed from its embedded `SKILL.md`), the `/rust` and `/orchestrate`
commands, and the standing auto-orchestrate guidance. Catalog assembly, scope precedence, command
parsing, argument expansion, and skill resolution stay in the crate root.

## Interface

`builtin_skills`, `builtin_rust_command`, and `builtin_orchestrate_command` are `pub(crate)` and
consumed only by catalog assembly (`Catalog::insert_builtin_skills` / `insert_command`), which is
also where user and project definitions of the same name override them.
`orchestrate_system_guidance` remains the crate's public export and is re-exported unchanged, so
`forge-core`'s auto-orchestrate injection is untouched.

## Why the prose belongs here

For these entries the text *is* the behaviour: the `/orchestrate` body and the standing guidance
are what the model reads at runtime. Keeping them in one owner makes the shipped contract
reviewable as a unit rather than as string literals scattered through catalog assembly.

## Characterization

The crate's existing tests cover the builtins end to end — that `/orchestrate` exists with no
import, that the built-in Rust skill loads with no filesystem access, and that `/rust` injects the
skill's methodology — and they continue to exercise them through the crate root's re-exports.
