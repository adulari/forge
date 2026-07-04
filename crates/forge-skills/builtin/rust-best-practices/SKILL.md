---
name: rust-best-practices
description: >-
  Write, review, or refactor Rust so it is idiomatic, safe, and maintainable. Use this
  skill whenever the task involves Rust code — implementing a feature in a `.rs` file,
  reviewing a Rust PR or module, refactoring a crate, designing a public API, fixing
  clippy or borrow-checker complaints, deciding how to structure error handling, or
  splitting an oversized function or file. Trigger even when the user does not say
  "best practices" explicitly: any request to "write this in Rust", "clean up this Rust
  code", "is this idiomatic", "add a Cargo crate", "handle these errors properly", or
  "make this compile without unwrap" should pull in this skill. Covers ownership and
  borrowing, error handling (Result / `?` / thiserror / anyhow), API design, module and
  crate layout, size limits, unsafe, dependencies, testing, and the verification steps
  (fmt, clippy, test) that must pass before the work is called done.
tier: standard
---

# Rust Best Practices

Idiomatic Rust is not a style preference — it is how you get the compiler and the
ecosystem to catch your mistakes for you. The goal of every guideline here is to move a
class of bug from "found in production" to "found at compile time" or "found by
`cargo clippy`". When you apply a rule, understand which failure it prevents; that lets
you know when the rule genuinely doesn't apply.

## Before you write code

Read the surrounding module first. A crate has a texture — its error type, its naming, how
it splits modules, whether it leans on iterators or explicit loops. Match it. A locally
"more correct" pattern that fights the existing conventions is worse than a consistent one,
because the next reader now has to hold two mental models.

## The verification gate (non-negotiable)

Rust code is not done until these pass. Run them and fix what they report — do not report
success on unverified code.

```
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

`cargo fmt` removes all whitespace and layout debate. `clippy` with `-D warnings` is the
single highest-leverage habit in Rust: it catches needless clones, non-idiomatic patterns,
likely bugs, and performance traps before review. If a lint is genuinely wrong for a case,
`#[allow(clippy::x)]` it *locally* with a one-line reason — never blanket-allow at crate
level, which silences future real hits.

## Error handling

This is where most non-idiomatic Rust shows up. The rules:

- **Return `Result`, propagate with `?`.** Don't `match` a `Result` just to re-return it.
  `?` is the idiom; it also converts error types through `From`, which is the whole point.
- **`unwrap` / `expect` / `panic!` are for "this cannot fail and if it does the program is
  broken".** In library code and any path driven by external input, they are bugs waiting
  to happen. `expect("reason")` beats `unwrap()` because the message documents the invariant
  and shows up in the panic. Reserve bare `unwrap()` for tests and truly impossible cases.
- **Applications vs libraries pick different error crates:**
  - *Application / binary*: `anyhow::Result<T>` with `.context("what failed")` at each
    boundary. You want a readable error chain, not a typed taxonomy the caller matches on.
  - *Library*: a typed error enum via `thiserror`, so callers can match on variants and your
    error is `std::error::Error`. Libraries should not force `anyhow` on their users.
- **Errors are meaningful and well-behaved** (Rust API Guideline C-GOOD-ERR): implement
  `Debug`, `Display`, and `Error`; carry enough context to act on; never stringify away
  structure the caller needs.
- Don't swallow errors with `let _ =` or `.ok()` unless ignoring them is genuinely correct —
  and if it is, a short comment says why.

## Ownership, borrowing, and types

- **Borrow, don't clone, to satisfy the borrow checker.** A `.clone()` added only to make an
  error go away is a smell — usually the fix is `&`, a lifetime, or restructuring. Clone when
  you genuinely need an owned copy, not as a borrow-checker escape hatch.
- **Accept the most general type; return the most specific.** Take `&str` not `&String`,
  `&[T]` not `&Vec<T>`, `impl AsRef<Path>` not `&PathBuf`. Return concrete types the caller
  can use directly.
- **Make illegal states unrepresentable.** Encode invariants in the type system: a newtype
  (`struct UserId(u64)`) instead of a bare `u64`; an `enum` instead of a `bool` + `Option`
  combo that has meaningless states. Arguments should convey meaning through types, not
  positional `bool`s (API Guideline C-CUSTOM-TYPE).
- **Prefer iterators and combinators** (`map`, `filter`, `collect`, `?`-in-iterator via
  `collect::<Result<_,_>>()`) over manual index loops — they are clearer and eliminate
  off-by-one and bounds bugs. Don't force it, though: a plain `for` loop with side effects is
  fine when a combinator chain would be more obscure.
- **Derive the common traits** (`Debug`, `Clone`, `PartialEq`, and `Eq`/`Hash`/`Default`/
  `Copy` where they make sense). Every public type should implement `Debug` (C-DEBUG).

## API design (public interfaces)

- Follow the naming conventions: `as_`/`to_`/`into_` for conversions by cost, `iter`/
  `iter_mut`/`into_iter` for iterators, RFC 430 casing.
- Keep struct fields private and expose behavior through methods, so you can change internals
  without a breaking release (C-STRUCT-PRIVATE).
- Use the **builder pattern** for types with many optional fields rather than a telescoping
  constructor or a giant `Option`-filled `new`.
- Implement standard conversion traits (`From`, `TryFrom`, `AsRef`) instead of ad-hoc
  `to_x`/`from_x` free functions; `From` gives you `Into` for free and plugs into `?`.
- Document every public item with a rustdoc comment, and include a `# Errors` section for
  fallible functions and a `# Panics` section for ones that can panic. Examples in docs are
  compiled and tested — they double as regression tests.

## Size and structure

These are conventions, not compiler limits — but they track real maintainability, and
clippy enforces the function one by default.

| Scope             | Practical target        | Basis |
|-------------------|-------------------------|-------|
| Function / method | **≤ 100 lines**         | clippy `too_many_lines` default threshold is 100. |
| `.rs` file / module | **≤ ~1,000 lines**    | No compiler/clippy limit; a common team convention. Split by responsibility. |
| Crate             | **No line cap**         | Split around a real API / domain / dependency boundary, not a line count. |
| Published `.crate`| **≤ 10 MB compressed**  | crates.io hard limit on the packaged archive. |

Exceptions that legitimately blow past the file target: generated code, large lookup
tables/fixtures, and macro-heavy modules. When a function crosses ~100 lines, the fix is
almost always to extract a well-named helper, not to `#[allow]` the lint — the name you give
the extracted piece is documentation.

Organize modules by responsibility, not by type-kind (`mod users` beats a `mod structs`).
Keep the crate's public surface in `lib.rs`/`mod.rs` re-exports so callers have one obvious
import path.

## Unsafe

- Default to safe Rust. Reach for `unsafe` only for FFI, genuine performance-critical spots
  proven by measurement, or building a safe abstraction over a raw operation.
- Every `unsafe` block gets a `// SAFETY:` comment stating the invariant that makes it sound.
  Keep unsafe blocks minimal and wrapped in a safe API so the unsafety doesn't leak.

## Dependencies

- Add a dependency when it earns its keep; each one is compile time, audit surface, and a
  future upgrade. Prefer well-maintained, widely-used crates.
- Use precise-enough version requirements, keep `Cargo.lock` committed for binaries, and run
  `cargo update` deliberately. Consider `cargo deny` / `cargo audit` for license and
  vulnerability checks on anything shipped.
- Fill in `Cargo.toml` metadata for published crates (description, license, repository,
  keywords, categories) — C-METADATA.

## Testing

- Unit tests live in a `#[cfg(test)] mod tests` beside the code; integration tests go in
  `tests/`. Test behavior and edge cases, not just the happy path.
- Prefer table-driven tests for multiple input/output cases. Use `assert_eq!` with meaningful
  values so failures are legible.
- Doctests keep examples honest — put a runnable example on public functions.
- For parsing/serialization and anything with an input space, consider property tests
  (`proptest`/`quickcheck`).

## Async (when it applies)

- Don't block in async code (`std::fs`, `std::thread::sleep`, long CPU loops) — it stalls the
  executor. Use the async equivalents or `spawn_blocking`.
- Don't hold a `std::sync::Mutex` guard across an `.await`. Use an async-aware lock or drop the
  guard first.
- Don't add `async` to a function that never awaits.

## Applying this in a review

When reviewing Rust rather than writing it, scan in this order — it surfaces the highest-value
issues first: (1) `unwrap`/`expect`/`panic!` on fallible or input-driven paths; (2) error
handling that discards context or matches-to-re-return instead of `?`; (3) needless `clone`s
and overly-specific parameter types; (4) missing `Debug`/derives on public types and missing
docs on public items; (5) oversized functions that should be split; (6) any `unsafe` without a
`// SAFETY:` justification. Point to `path:line`, say which failure the issue invites, and
prefer suggesting the idiomatic shape over just naming the rule.
