//! The hard scenario: about 40 turns with large tool outputs under a low auto-compaction
//! ceiling, so the real auto-compact, floor-prune and window-trim path fires repeatedly.
//!
//! On top of the standard fact kinds it plants values that only ever appear in a tool result,
//! a project rule that lives in AGENTS.md (probed by a work task long after the file was read),
//! a mid-session correction that overrides an earlier instruction, an `update_tasks` list whose
//! statuses change, and a work-state question scored against the real `git status`.

use super::{ask, m, turn, Category, Check, Matcher, Probe, Turn, Zone};

const PENDING_STATE: &[Matcher] = &[m(&["test", "empty"]), m(&["0\\.4\\.2"]), m(&["changelog"])];

pub(super) static HARD_TURNS: &[Turn] = &[
    Turn {
        zone: Zone::Plant,
        prompt: "We are working on hdrparse, a small HTTP header parser crate in this directory. \
Two house rules for the whole session. First, every commit message must start with JIRA-123 (for \
example `JIRA-123: fix header limit`); we are not committing yet. Second, nothing under data/ may \
ever be edited, those files are produced by a generator. Read src/parser.rs and tell me in one \
sentence what parse_header does.",
        probes: &[
            Probe {
                id: "commit_prefix",
                category: Category::Constraint,
                prompt: "What prefix must every commit message start with?",
                check: ask(&["JIRA-123"]),
                good: "Every commit message must start with JIRA-123.",
                bad: "Use conventional commits like feat: or fix:.",
            },
            Probe {
                id: "data_readonly",
                category: Category::Constraint,
                prompt: "Which directory did I say must never be edited, and why?",
                check: ask(&["\\bdata\\b", "generat|tool"]),
                good: "Nothing under data/ may be edited because the files are generated.",
                bad: "You said not to touch the tests directory.",
            },
        ],
    },
    turn(
        Zone::Plant,
        "For new Rust code you write, use 4 spaces for indentation. Read src/lexer.rs and name the \
token kinds it defines, in one sentence.",
    ),
    Turn {
        zone: Zone::Plant,
        prompt: "Decision: retries use exponential backoff with jitter, capped at 30 seconds, \
because the upstream vendor rate-limits us at 10 requests per second. Read src/server.rs and tell \
me what handle_request returns, in one sentence.",
        probes: &[Probe {
            id: "backoff_decision",
            category: Category::Decision,
            prompt: "What retry policy did we decide on, what is its cap, and why that design?",
            check: ask(&["jitter", "\\b30\\s*(s\\b|sec)", "rate.?limit|10 (requests|req)"]),
            good: "Exponential backoff with jitter capped at 30 seconds because the vendor rate-limits at 10 requests per second.",
            bad: "Fixed one-second retries.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Add a function `retry_budget(attempts: u32) -> u32` to src/limits.rs that returns \
attempts capped at 5. Confirm in one sentence.",
        probes: &[Probe {
            id: "added_function",
            category: Category::Location,
            prompt: "Which function did I ask you to add, and to which file?",
            check: ask(&["retry_budget", "limits\\.rs"]),
            good: "retry_budget, in src/limits.rs.",
            bad: "A parse_port function in server.rs.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Run `bash scripts/check.sh` and show me exactly what it prints. Do not fix \
anything yet.",
        probes: &[Probe {
            id: "error_code",
            category: Category::ErrorFix,
            prompt: "What error code did scripts/check.sh print when we first ran it?",
            check: ask(&["E-4172"]),
            good: "It printed ERROR E-4172 about the header length limit.",
            bad: "It printed E-1001 and a segfault.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Fix that by raising MAX_HEADER_BYTES to 16384 in src/limits.rs, then re-run \
scripts/check.sh and confirm it passes.",
        probes: &[Probe {
            id: "error_fix_value",
            category: Category::ErrorFix,
            prompt: "What value did we raise MAX_HEADER_BYTES to in order to fix that error?",
            check: ask(&["16[,_]?384|16\\s?kib"]),
            good: "We raised MAX_HEADER_BYTES to 16384.",
            bad: "We lowered it to 4096.",
        }],
    },
    turn(
        Zone::Plant,
        "Use the update_tasks tool to track three tasks, all pending for now: `port lexer \
fixtures`, `audit retry budget`, `write migration notes`. Confirm in one sentence.",
    ),
    Turn {
        zone: Zone::Plant,
        prompt: "For context, two numbers. Our staging server listens on port 47123, and our \
latency target is p99 under 230 ms. Do not change anything, just confirm in one sentence.",
        probes: &[
            Probe {
                id: "staging_port",
                category: Category::Number,
                prompt: "What port did I say the staging server listens on?",
                check: ask(&["47123"]),
                good: "Port 47123.",
                bad: "Port 8080.",
            },
            Probe {
                id: "latency_target",
                category: Category::Number,
                prompt: "What p99 latency target did I mention?",
                check: ask(&["\\b230\\s*ms"]),
                good: "p99 under 230 ms.",
                bad: "p99 under 100 ms.",
            },
        ],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Two things that are still TODO, do NOT do either now, just remember them. One: \
add a regression test for the empty-header case in tests/header_empty.rs. Two: bump the crate \
version to 0.4.2 once the limits change has landed. Confirm you have noted both, in one sentence.",
        probes: &[
            Probe {
                id: "pending_test",
                category: Category::Pending,
                prompt: "What test did I say still needs to be written later, not now?",
                check: ask(&["test", "empty"]),
                good: "A regression test for the empty-header case.",
                bad: "A load test for the server.",
            },
            Probe {
                id: "pending_version",
                category: Category::Pending,
                prompt: "What version did I say to bump the crate to once the limits change lands?",
                check: ask(&["0\\.4\\.2"]),
                good: "Bump it to 0.4.2.",
                bad: "Bump it to 1.0.0.",
            },
        ],
    },
    turn(
        Zone::Plant,
        "Next feature: a loader for our runtime settings. It should read config.toml from the \
project root. Look at src/config.rs and tell me which public functions it has, in one sentence.",
    ),
    Turn {
        zone: Zone::Plant,
        prompt: "Correction: no, the config file is settings.toml, not config.toml. Do not change \
any code, just confirm in one sentence.",
        probes: &[Probe {
            id: "config_filename",
            category: Category::Correction,
            prompt: "What is the file name of the config file the loader should read?",
            check: ask(&["settings\\.toml"]),
            good: "The loader should read settings.toml (not config.toml).",
            bad: "It should read config.toml from the project root.",
        }],
    },
    Turn {
        zone: Zone::Work,
        prompt: "Use the read_file tool to open the whole of data/region_table.csv (not grep, wc or head), then \
tell me only how many rows it has. One sentence.",
        probes: &[Probe {
            id: "region_quota",
            category: Category::ToolResult,
            prompt: "In the region table you read earlier, what quota was listed for region eu-west-9?",
            check: ask(&["7311"]),
            good: "eu-west-9 had quota 7311.",
            bad: "eu-west-9 had quota 4200.",
        }],
    },
    Turn {
        zone: Zone::Work,
        prompt: "Run `bash scripts/run_checks.sh` and tell me only whether the suite passed \
overall. One sentence.",
        probes: &[Probe {
            id: "check_output",
            category: Category::ToolResult,
            prompt: "In the run_checks.sh output you saw earlier, what checksum did case_137 expect?",
            check: ask(&["BEEF42"]),
            good: "case_137 expected 0xBEEF42.",
            bad: "case_137 got 0xC0FFEE.",
        }],
    },
    turn(
        Zone::Work,
        "Use read_file to open all of data/events.log (not grep or wc) and tell me how many lines contain level=ERROR. One sentence.",
    ),
    turn(
        Zone::Work,
        "Use read_file to open all of docs/NOTES.md (not grep or wc) and quote its first heading. One sentence.",
    ),
    turn(
        Zone::Work,
        "Use read_file to open all of src/tables.rs (not grep or wc) and tell me the name of its last constant. One sentence.",
    ),
    turn(
        Zone::Work,
        "Run `wc -l src/*.rs data/* docs/*.md` and tell me which file is longest. One sentence.",
    ),
    turn(
        Zone::Work,
        "Use read_file to open all of data/status_codes.txt (not grep or wc) and tell me how many entries have retryable=true. One sentence.",
    ),
    turn(
        Zone::Work,
        "Search the repo for the word TODO (grep -rn) and tell me how many matching lines there \
are. One sentence.",
    ),
    turn(
        Zone::Work,
        "Use read_file to open all of data/events.log again (not grep or wc) and tell me how many lines contain level=WARN. One sentence.",
    ),
    turn(
        Zone::Work,
        "Run `cat src/parser.rs src/lexer.rs src/server.rs src/config.rs` and count the `fn` \
items across them. One sentence.",
    ),
    turn(
        Zone::Work,
        "Use read_file to open all of docs/NOTES.md (not grep or wc) and tell me how many `## Note` sections it has. One sentence.",
    ),
    turn(Zone::Marker, "mid-scenario"),
    Turn {
        zone: Zone::Late,
        prompt: "Change of plan: from now on use tabs for indentation in new Rust code, not \
spaces. Just confirm in one sentence.",
        probes: &[Probe {
            id: "indent_override",
            category: Category::Correction,
            prompt: "What indentation style did I most recently ask for in new Rust code?",
            check: ask(&["\\btabs?\\b"]),
            good: "Tabs, not spaces.",
            bad: "4 spaces.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Use the update_tasks tool to mark `port lexer fixtures` and `audit retry budget` \
as done and leave `write migration notes` open. Confirm in one sentence.",
        probes: &[Probe {
            id: "tasks_open",
            category: Category::TaskList,
            prompt: "Which tasks are still open on the task list, not done? Reply with just their titles.",
            check: Check::Answer(Matcher {
                all: &["migration notes"],
                none: &["lexer fixtures", "audit retry"],
            }),
            good: "write migration notes",
            bad: "port lexer fixtures, audit retry budget, write migration notes",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "One more thing that is still left, do NOT do it now: add a changelog section \
to docs/NOTES.md. Just confirm in one sentence.",
        probes: &[Probe {
            id: "pending_changelog",
            category: Category::Pending,
            prompt: "What documentation change did I say is still left to do later?",
            check: ask(&["changelog", "notes\\.md"]),
            good: "A changelog section in docs/NOTES.md.",
            bad: "Rewrite the README.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Heads up: the production rollout window is 03:15 UTC. Do not change anything, \
just confirm in one sentence.",
        probes: &[Probe {
            id: "rollout_window",
            category: Category::Number,
            prompt: "What time did I say the production rollout window is?",
            check: ask(&["03:15"]),
            good: "03:15 UTC.",
            bad: "21:00 UTC.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Decision: the lexer stays ASCII-only, and any non-ASCII byte in a header is \
rejected with an error, because RFC 9110 field names are ASCII tokens. Open src/lexer.rs and \
tell me where bytes are classified, in one sentence.",
        probes: &[Probe {
            id: "ascii_decision",
            category: Category::Decision,
            prompt: "What did we decide about non-ASCII bytes in the lexer, and why?",
            check: ask(&["ascii", "reject|error|disallow|not allowed|refuse"]),
            good: "The lexer is ASCII-only and rejects non-ASCII bytes because field names are ASCII tokens.",
            bad: "The lexer should decode UTF-8 transparently.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Rename the helper `tok_next` to `next_token` in src/lexer.rs, including every \
call site in the repo. Confirm in one sentence.",
        probes: &[
            Probe {
                id: "renamed_helper",
                category: Category::Location,
                prompt: "What did the helper tok_next get renamed to, and in which file?",
                check: ask(&["next_token", "lexer\\.rs"]),
                good: "tok_next became next_token in src/lexer.rs.",
                bad: "It was renamed to read_token in parser.rs.",
            },
            Probe {
                id: "work_state",
                category: Category::WorkState,
                prompt: "Which files have you changed so far in this repo, and what is still left \
to do? List the file paths and the remaining items.",
                check: Check::WorkState {
                    pending: PENDING_STATE,
                },
                good: "I changed src/limits.rs and src/lexer.rs. Still left: a regression test for \
the empty-header case, bumping to 0.4.2, and a changelog section in docs/NOTES.md.",
                bad: "Nothing is left to do.",
            },
        ],
    },
    turn(
        Zone::Wrap,
        "Use read_file to open all of data/events.log (not grep or wc) and quote the first line whose level is ERROR. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of src/tables.rs (not grep or wc) and tell me the value of TABLE_ENTRY_100. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of docs/NOTES.md (not grep or wc) and quote the heading of Note 33. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of data/status_codes.txt (not grep or wc) and tell me the reason text of entry 0042. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Run `grep -c level=INFO data/events.log` and tell me the count. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of data/events.log (not grep or wc) and tell me how many distinct component= values appear. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Run `wc -c src/*.rs docs/*.md` and tell me the total size. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of src/tables.rs (not grep or wc) and tell me how many constants it defines. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of docs/NOTES.md (not grep or wc) and tell me which note mentions topic 7. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Use read_file to open all of data/status_codes.txt (not grep or wc) and tell me how many entries have retryable=false. One sentence.",
    ),
    turn(Zone::Marker, "near-end"),
];

/// Work tasks whose result is read off the files the model wrote. The rules they test were
/// stated long before: AGENTS.md (read at session start) and a user correction.
pub(super) static HARD_BEHAVIOR: &[Probe] = &[
    Probe {
        id: "obey_agents_doc_rule",
        category: Category::Behavior,
        prompt: "Add a function `retry_after_secs(raw: &str) -> Option<u32>` to src/server.rs \
that parses a Retry-After header value given in whole seconds. Confirm in one sentence.",
        check: Check::File {
            path: "src/server.rs",
            matcher: Matcher {
                all: &["^[ \\t]*///\\s*Forge:[^\\n]*\\n(?:[ \\t]*///[^\\n]*\\n|[ \\t]*#\\[[^\\n]*\\n)*[ \\t]*(pub\\s+)?fn\\s+retry_after_secs"],
                none: &[],
            },
        },
        good: "/// Forge: parses a Retry-After value.\npub fn retry_after_secs(raw: &str) -> Option<u32> {\n\traw.trim().parse().ok()\n}\n",
        bad: "pub fn retry_after_secs(raw: &str) -> Option<u32> {\n\traw.trim().parse().ok()\n}\n",
    },
    Probe {
        id: "obey_latest_indent",
        category: Category::Behavior,
        prompt: "Create src/digits.rs with a function `count_digits(s: &str) -> usize` that \
counts the ASCII digits in s. Confirm in one sentence.",
        check: Check::File {
            path: "src/digits.rs",
            matcher: Matcher {
                all: &["fn count_digits", "^\\t+\\S"],
                none: &["^ +\\S"],
            },
        },
        good: "/// Forge: counts digits.\npub fn count_digits(s: &str) -> usize {\n\ts.bytes().filter(|b| b.is_ascii_digit()).count()\n}\n",
        bad: "pub fn count_digits(s: &str) -> usize {\n    s.bytes().filter(|b| b.is_ascii_digit()).count()\n}\n",
    },
    Probe {
        id: "obey_commit_prefix",
        category: Category::Behavior,
        prompt: "Commit all the current changes to git now.",
        check: Check::CommitSubject(Matcher {
            all: &["^JIRA-123"],
            none: &[],
        }),
        good: "JIRA-123: raise header limit",
        bad: "fix: raise header limit",
    },
];

pub(super) static HARD_EXTRAS: &[&str] = &[
    "Use read_file to open all of data/events.log (not grep or wc) and tell me the last line's timestamp. One sentence.",
    "Use read_file to open all of src/tables.rs (not grep or wc) and tell me the value of TABLE_ENTRY_007. One sentence.",
    "Use read_file to open all of docs/NOTES.md (not grep or wc) and quote the heading of Note 12. One sentence.",
    "Run `wc -l data/*` and tell me the line count of events.log. One sentence.",
];
