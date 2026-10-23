# Turn contract: when a prompt makes the turn read-only

A turn can carry a one-line contract that is injected as a system message: read-only, planning
only, or "this request requires an implementation". The read-only contract is the expensive one.
It denies shell and every write for the whole turn, and the model is told to change nothing. When
it is inferred from a prompt that actually asked for a fix, the turn ends with a diagnosis and no
edit. Code: `crates/forge-core/src/turn_contract.rs` (the contract),
`crates/forge-core/src/turn_contract/read_only.rs` and
`crates/forge-core/src/turn_contract/read_only/prohibition.rs` (the classifier).

## The rule

A prompt is read-only only when it says so explicitly, and a prohibition says so only when it names
no part of the tree. Doubt resolves to "not read-only": a missing contract costs a guarantee, a
wrong one costs the task.

| Reads as read-only | Reads as a constraint on a change request |
|---|---|
| `Do not edit anything.` `Don't change any files.` `Make no edits.` `Answer without changing anything.` | `Do not modify anything under logs/, ops/ or data/, nor README.md, ...` |
| `Don't touch anything on disk.` `Do not edit any file in the project.` `Do not change code or files.` | `Don't edit the incoming CSV.` `Do not edit, delete, or rename the existing tests.` |
| `Don't edit the code, just explain the flow.` `Do not edit it in any way.` `Don't edit files yet.` | `Do not edit files under tests/.` `Don't change anything else.` `Don't edit anything except CHANGELOG.md.` |
| `Read-only: inspect the parser.` `This is a read-only question: ...` `Treat this as read-only.` | `Read-only files: data/, logs/.` `This is read-only data under fixtures/.` |
| `Explain how it works. Don't change the code.` | `Update the docs. Don't touch the code.` |

How a prohibition is read:

- It starts with `do not` / `don't` (or `without`, `make no`) and a write verb (`edit`, `modify`,
  `change`, `touch`, `alter`, `make changes`, `write files`), optionally followed by coordinated
  verbs (`edit, delete, or rename`).
- The object has to be one of a short list that names the whole tree: `anything`, `files`,
  `any file`, or, when nothing else in the prompt asks for a change, `code`, `the repo`, `the
  project`, `it`. A named path, a backtick-quoted token, a list, `the existing tests`, `the file`
  and every object the classifier does not recognise make the prohibition a constraint.
- A tail that narrows the object (`under`, `in tests/`, `outside`, `other than`, `except`, `else`,
  `unrelated`, `that ...`) makes it a constraint too. A tail that restates the ban (`at all`,
  `whatsoever`, `in any way`, `in place`, `yet`, `for now`, `in the repo`) leaves it universal. So
  does a clause that turns to answering (`, just explain`, `and tell me what you find`).
- A colon or a bullet list after the verb (`Do not edit:` then `- a.py`) introduces protected paths.
- "Next to a request to change" matters only for the ambiguous objects (`the code`, `the repo`, `it`):
  `Update the docs. Don't touch the code.` is a change request, `Explain it. Don't touch the code.`
  is not. `anything` and `any files` are explicit and are never overruled.
- `read-only` as a bare word counts only when it opens a sentence, and not when it qualifies a
  thing: `Read-only files:`, `read-only access`, `read-only mounts`.

A peer agent's message never narrows the receiving session's permissions, whatever it says.

## Tests

`crates/forge-core/src/turn_contract/read_only_tests.rs` holds two tables of about a hundred
phrasings each (constraints and descriptions that must not be read-only, requests that must be),
two cross-product tests that combine every way of forbidding an edit with every kind of object and
every way of framing the request, and a scrambled-input test that guards the slicing.
