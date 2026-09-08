# Feature: ask_user — interactive questions the agent asks mid-task

> Status: **SHIPPED** (single question 2026-06-15; multi-question form with multi-select, free
> text and notes 2026-09-08). Code: `crates/forge-core/src/session_virtual_tools.rs`
> (`ask_user`, `parse_questions`, `render_answers`), `crates/forge-tui/src/question_form.rs`
> (state machine), `crates/forge-tui/src/app/render/question.rs` (TUI), `mobile/src/components/
> cards/QuestionCard.tsx` (companion app).

## 1. Problem

The agent hits a fork only the user can resolve — a value choice, a missing requirement, which
of several valid designs. Guessing is expensive; a dozen one-line "which one?" round trips are
worse. The tool should let the model ask everything it needs in one go, with good suggested
answers, and the surface should make answering feel like a form, not a quiz typed into the chat
line.

## 2. The tool

`ask_user` is a core-owned virtual tool (it needs the presenter, which ordinary tools cannot
reach). Two argument shapes are accepted:

```jsonc
// the form
{ "questions": [
    { "header": "Database",                      // ≤ 16 chars, shown as the tab label
      "question": "Which database should sessions use?",
      "options": [ { "label": "Postgres", "description": "recommended — already provisioned" },
                   { "label": "SQLite",   "description": "single file, no ops" } ],
      "multi_select": false,                     // default false
      "allow_other": true,                       // default true; forced when there are no options
      "allow_note": true },                      // default true
    { "header": "Scope", "question": "Which features go in the MVP?", "multi_select": true,
      "options": [ { "label": "Auth" }, { "label": "Billing" }, { "label": "Search" } ] }
] }

// the original single-question shorthand (still accepted; old transcripts and models emit it)
{ "question": "…", "options": [ … ], "allow_other": true }
```

Limits: 1–6 questions per call, each with non-empty text. Malformed calls return an
`error: ask_user: …` tool result instead of blocking the turn.

**Result** the model reads back:

- one question → one line: `Postgres — note: managed, please`, or `Auth, Billing, other: SSO`
  for a multi-select with a typed extra;
- several → a numbered list, one line per question, labelled by header (or the question text):
  ```
  1. Database: Postgres — note: managed, please
  2. Scope: Auth, Billing, other: SSO
  ```
- dismissed / non-interactive → `(no answer — non-interactive)` (the original sentinel), so the
  model proceeds instead of hanging.

## 3. Surfaces

### TUI (`forge chat`)

The form takes the live region while open (like the pickers); the transcript above is untouched.

```
  ❓ question 2 of 3    ✓ Database   ▸ Scope   · Deploy
     Which features go in the MVP?  (choose all that apply)

     ▸ ☑ 1 Auth        login, sessions
       ☐ 2 Billing     stripe
       ☑ 3 Search
       ○ Other…        type your own answer
       ✎ note          optional — anything the model should know
```

| key | does |
|---|---|
| `↑` `↓` / `j` `k` | move between rows |
| `1`–`9` | pick that option (single-select: also moves on) |
| `Space` | toggle the option (multi-select) / select it / open Other or note when on that row |
| `Enter` | single-select: choose the highlighted option and move on · multi-select: confirm and move on · on **Other…** / **note**: start typing; `Enter` again saves |
| `o` / `n` | jump straight into Other… / the note |
| `←` `→` / `Shift-Tab` `Tab` | previous / next question (answers are kept) |
| `Esc` | while typing: leave the field · otherwise dismiss the whole form (no answer) |

Enter on the last question submits. If an earlier question was skipped, the form parks on it and
says so instead of sending a half-empty answer. A single-select question with a typed "Other"
replaces the chosen option; a multi-select keeps both. The statusline shows the keys that apply
to the current row. When the form closes, each question and its answer are written to scrollback
(`❓ … ↳ Postgres — note: …`) so the record survives.

### Companion app / web (`forge serve`)

The snapshot carries the whole form (`question_form`, protocol v11 additive) next to the legacy
`question` / `question_options` fields, which mirror the *current* question so older clients keep
working. The `QuestionCard` renders a stepper for several questions, radio / checkbox rows,
an "other" input, an optional note, and one **send answers** button. It answers once, as
`answer{ text: JSON.stringify({ "answers": [ { "selected": [...], "other": …, "note": … }, … ] }),
seq }`, and locks until a new `prompt_seq` arrives (ARCHITECTURE.md §3). Against a pre-v11 host
it falls back to the original single-question card and answers the 1-based option number.

A legacy plain-text `answer` (an option number or free text) is still accepted by the host: it
answers the current question and, on a multi-question form, advances to the next one.

### Headless (`forge run` on a TTY)

Each question prints with numbered options; a single number (or `1,3` on a multi-select) picks,
anything else is the free-text answer when allowed, then `note (Enter to skip):`. Three invalid
tries → no answer. Without a TTY the tool returns the sentinel immediately.

### Other presenters

`Presenter::ask_form` has a default implementation that asks each question through the older
`Presenter::ask` (label or free text), so MCP / stream-json / test presenters keep working; they
lose multi-select and notes but never block.

## 4. Plan approval

`present_plan`'s "Build this plan?" still rides the single-question path (`App::set_question` →
a one-question form), so the same keys answer it: `Enter` builds, `2` cancels, `o` types changes.

## 5. Tests

- `session_virtual_tools::ask_user_form_tests` — both argument shapes, limits, result rendering.
- `question_form::tests` — every key path above, remote text and structured answers.
- `app::tests` — the form renders in the live region and never looks like a permission prompt;
  the remote snapshot suppresses `permission_prompt` while a question is active.
- `serve.rs` plan-approval e2e — the page's `Answer("1")` still approves.
