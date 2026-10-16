//! The recall scenario as data: every user turn, the facts it plants, and the probe that checks
//! each fact. Adding a fact is one `Probe` on the turn that states it (plus a matcher with a
//! known-good and known-bad sample, which the unit tests run through the scorer).
//!
//! Order of execution: `Plant` turns, `Work` turns (scaled by `--turns-scale`), compaction #1,
//! `Late` turns, `Wrap` turns (scaled), compaction #2, then probes, then a resume and the same
//! probes again.

use serde::Serialize;

mod hard;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Category {
    Constraint,
    Correction,
    Decision,
    Location,
    ErrorFix,
    Pending,
    Number,
    Behavior,
    /// A value that was only ever visible in a tool result (a file read, command output).
    ToolResult,
    /// "Which files have you changed and what is left?", scored against the real git state.
    WorkState,
    /// The `update_tasks` list and which tasks are still open.
    TaskList,
}

impl Category {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Constraint => "constraint",
            Self::Correction => "correction",
            Self::Decision => "decision",
            Self::Location => "location",
            Self::ErrorFix => "error_fix",
            Self::Pending => "pending",
            Self::Number => "number",
            Self::Behavior => "behavior",
            Self::ToolResult => "tool_result",
            Self::WorkState => "work_state",
            Self::TaskList => "task_list",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Zone {
    /// Early facts: folded by both compactions.
    Plant,
    /// Cheap tool work that grows the context before compaction #1.
    Work,
    /// Facts planted after compaction #1: they test a summary of a summary.
    Late,
    /// Tool work that pushes the late facts out of the kept tail before compaction #2.
    Wrap,
    /// Not a user turn: the prompt is the label of a manual compaction at this point.
    Marker,
}

impl Zone {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Plant => "early",
            Self::Work | Self::Wrap => "work",
            Self::Marker => "marker",
            Self::Late => "late",
        }
    }
}

/// A deterministic pass/fail rule over text: every `all` regex must match and no `none` regex may.
/// Regexes are case-insensitive and unanchored.
#[derive(Debug, Clone, Copy)]
pub(super) struct Matcher {
    pub all: &'static [&'static str],
    pub none: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Check {
    /// The model's reply to a "from memory, no tools" question.
    Answer(Matcher),
    /// The model was asked to do work; the file it wrote must satisfy the matcher.
    File {
        path: &'static str,
        matcher: Matcher,
    },
    /// The model was asked to commit; a new commit must exist and its subject must match.
    CommitSubject(Matcher),
    /// "Which files have you changed and what is left?" Two verdicts: every file in the actual
    /// `git status` is named in the reply, and every pending matcher hits.
    WorkState { pending: &'static [Matcher] },
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Probe {
    pub id: &'static str,
    pub category: Category,
    /// A question (asked from memory) for `Check::Answer`, a task for the other checks.
    pub prompt: &'static str,
    pub check: Check,
    /// A reply (or file/subject text) the matcher must accept, and one it must reject.
    #[cfg_attr(not(test), allow(dead_code))]
    pub good: &'static str,
    #[cfg_attr(not(test), allow(dead_code))]
    pub bad: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Turn {
    pub zone: Zone,
    pub prompt: &'static str,
    /// Facts this turn states, one probe each.
    pub probes: &'static [Probe],
}

const fn m(all: &'static [&'static str]) -> Matcher {
    Matcher { all, none: &[] }
}

const fn ask(all: &'static [&'static str]) -> Check {
    Check::Answer(m(all))
}

const fn turn(zone: Zone, prompt: &'static str) -> Turn {
    Turn {
        zone,
        prompt,
        probes: &[],
    }
}

/// Prefix of every `Check::Answer` probe prompt.
pub(super) const ASK_PREFIX: &str =
    "Memory check, answer from our conversation so far. Do not use \
any tools and do not open any files. Reply in one or two sentences. ";

pub(super) static STANDARD_TURNS: &[Turn] = &[
    Turn {
        zone: Zone::Plant,
        prompt: "We are working on hdrparse, a small HTTP header parser crate in this directory. \
House rule for this whole session: never use unwrap() anywhere in src/parser.rs, use ? or an \
explicit match instead. Now read src/parser.rs and tell me in one sentence what parse_header does.",
        probes: &[Probe {
            id: "unwrap_rule",
            category: Category::Constraint,
            prompt: "Which source file did I say must never use unwrap(), and what must be used instead?",
            check: ask(&["parser\\.rs", "unwrap"]),
            good: "src/parser.rs must never use unwrap(); use ? or a match.",
            bad: "You said to avoid panics in main.rs.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Second rule: every commit message in this repo must start with JIRA-123 (for \
example `JIRA-123: fix header limit`). We are not committing yet, just remember it. Run `ls src` \
and list the files you see.",
        probes: &[Probe {
            id: "commit_prefix",
            category: Category::Constraint,
            prompt: "What prefix must every commit message start with?",
            check: ask(&["JIRA-123"]),
            good: "Every commit message must start with JIRA-123.",
            bad: "Use conventional commits like feat: or fix:.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Two decisions. First: we parse with our own hand-rolled code and do NOT use the \
nom crate, because nom added about 40 seconds to a clean build and we need CI to stay fast. \
Second: timeouts in the config are stored as whole seconds in a u32, not as a Duration, because \
the config file is edited by humans and Duration does not deserialize nicely from toml. Open \
src/lexer.rs and name the token kinds it defines.",
        probes: &[
            Probe {
                id: "parser_choice",
                category: Category::Decision,
                prompt: "Which parsing approach did we choose over which alternative crate, and why?",
                check: ask(&["nom", "build|compile|\\bci\\b|40"]),
                good: "Hand-rolled, not nom, because nom slowed the clean build by 40s.",
                bad: "We picked the regex crate because it is fast.",
            },
            Probe {
                id: "timeout_type",
                category: Category::Decision,
                prompt: "What type did we decide to use for timeouts in the config, and why?",
                check: ask(&["u32", "human|edit|deserial|toml"]),
                good: "A u32 of whole seconds, because humans edit the toml and Duration does not deserialize well.",
                bad: "A Duration, since it is the standard type.",
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
        zone: Zone::Plant,
        prompt: "Add a function `validate_header_len(len: usize) -> Result<(), ParseError>` to \
src/limits.rs that returns an error when len is larger than MAX_HEADER_BYTES. Keep it short, then \
confirm in one sentence.",
        probes: &[Probe {
            id: "added_function",
            category: Category::Location,
            prompt: "Which function did I ask you to add, and to which file?",
            check: ask(&["validate_header_len", "limits\\.rs"]),
            good: "validate_header_len, in src/limits.rs.",
            bad: "You asked for a parse_port function in server.rs.",
        }],
    },
    Turn {
        zone: Zone::Plant,
        prompt: "Rename the helper `tok_next` to `next_token` in src/lexer.rs, including every \
call site in the repo. Confirm in one sentence.",
        probes: &[Probe {
            id: "renamed_helper",
            category: Category::Location,
            prompt: "What did the helper tok_next get renamed to, and in which file?",
            check: ask(&["next_token", "lexer\\.rs"]),
            good: "tok_next became next_token in src/lexer.rs.",
            bad: "It was renamed to read_token in parser.rs.",
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
    turn(
        Zone::Work,
        "Read data/status_codes.txt and tell me how many entries have retryable=true. One sentence.",
    ),
    turn(
        Zone::Work,
        "Read docs/NOTES.md and quote its first heading. One sentence.",
    ),
    turn(
        Zone::Work,
        "Read src/tables.rs and tell me the name of its last constant. One sentence.",
    ),
    turn(
        Zone::Work,
        "Run `wc -l src/*.rs data/*.txt docs/*.md` and tell me which file is longest. One sentence.",
    ),
    turn(
        Zone::Work,
        "Add the one-line doc comment `/// handles one request` directly above `fn handle_request` \
in src/server.rs and confirm in one sentence.",
    ),
    turn(
        Zone::Work,
        "Search the repo for the word TODO (grep -rn) and tell me how many matching lines there \
are. One sentence.",
    ),
    turn(Zone::Marker, "mid-scenario"),
    Turn {
        zone: Zone::Late,
        prompt: "New rule from here on: every new public function needs a `# Errors` section in \
its rustdoc. Just confirm in one sentence.",
        probes: &[Probe {
            id: "late_doc_rule",
            category: Category::Constraint,
            prompt: "What rustdoc section did I say every new public function must have?",
            check: ask(&["#?\\s*errors"]),
            good: "Every new public function needs a `# Errors` section.",
            bad: "A `# Examples` section.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Decision: the lexer stays ASCII-only, and any non-ASCII byte in a header is \
rejected with an error, because RFC 9110 field names are ASCII tokens. Open src/lexer.rs and \
tell me where bytes are classified, in one sentence.",
        probes: &[Probe {
            id: "late_ascii_decision",
            category: Category::Decision,
            prompt: "What did we decide about non-ASCII bytes in the lexer, and why?",
            check: ask(&["ascii", "reject|error|disallow|not allowed|refuse"]),
            good: "The lexer is ASCII-only and rejects non-ASCII bytes because field names are ASCII tokens.",
            bad: "The lexer should decode UTF-8 transparently.",
        }],
    },
    Turn {
        zone: Zone::Late,
        prompt: "Benchmark budget: the header benchmark must stay under 12 ms for 10,000 \
headers. Do not change anything, just confirm in one sentence.",
        probes: &[Probe {
            id: "late_bench_budget",
            category: Category::Number,
            prompt: "What time budget did I give the header benchmark?",
            check: ask(&["\\b12\\s*ms"]),
            good: "Under 12 ms for 10,000 headers.",
            bad: "Under 50 ms.",
        }],
    },
    turn(
        Zone::Wrap,
        "Read src/server.rs and tell me what handle_request returns. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Read data/status_codes.txt again and tell me the reason text of entry 0042. One sentence.",
    ),
    turn(
        Zone::Wrap,
        "Read docs/NOTES.md and src/tables.rs and tell me which of the two is longer. One sentence.",
    ),
    turn(Zone::Marker, "near-end"),
];

/// Probes that need the model to do work; run after the question probes so a stray edit cannot
/// leak into an answer.
pub(super) static STANDARD_BEHAVIOR: &[Probe] = &[
    Probe {
        id: "obey_unwrap_rule",
        category: Category::Behavior,
        prompt: "Add a function `parse_content_length(raw: &str) -> Result<usize, ParseError>` to \
src/parser.rs that parses a Content-Length header value. Confirm in one sentence.",
        check: Check::File {
            path: "src/parser.rs",
            matcher: Matcher {
                all: &["fn parse_content_length"],
                none: &["\\.unwrap\\s*\\("],
            },
        },
        good: "pub fn parse_content_length(raw: &str) -> Result<usize, ParseError> { raw.trim().parse().map_err(|_| ParseError::Bad) }",
        bad: "pub fn parse_content_length(raw: &str) -> Result<usize, ParseError> { Ok(raw.parse().unwrap()) }",
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

/// Filler turns for `--turns-scale` above 1: cycled with a pass number appended.
pub(super) static STANDARD_EXTRAS: &[&str] = &[
    "Read src/lexer.rs and count its `fn` items. One sentence.",
    "Read src/config.rs and name its first struct. One sentence.",
    "Read src/limits.rs and tell me every constant it defines. One sentence.",
    "Read README.md and quote its first line. One sentence.",
];

/// Which scenario `--scenario` selects.
pub(super) struct Scenario {
    pub name: &'static str,
    pub turns: &'static [Turn],
    /// Probes that need the model to do work; run after the question probes.
    pub behavior: &'static [Probe],
    /// Filler for `--turns-scale` above 1, cycled with a pass number appended.
    pub extras: &'static [&'static str],
    pub fixture: FixtureKind,
    /// Auto-compaction ceiling (tokens) written into the temp config, so the real auto path fires
    /// under pressure; `None` leaves the model's own ceiling.
    pub compact_cap: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FixtureKind {
    Standard,
    Hard,
}

pub(super) static STANDARD: Scenario = Scenario {
    name: "standard",
    turns: STANDARD_TURNS,
    behavior: STANDARD_BEHAVIOR,
    extras: STANDARD_EXTRAS,
    fixture: FixtureKind::Standard,
    compact_cap: None,
};

pub(super) static HARD: Scenario = Scenario {
    name: "hard",
    turns: hard::HARD_TURNS,
    behavior: hard::HARD_BEHAVIOR,
    extras: hard::HARD_EXTRAS,
    fixture: FixtureKind::Hard,
    compact_cap: Some(24_000),
};

impl Scenario {
    pub(super) fn by_name(name: &str) -> Option<&'static Scenario> {
        [&STANDARD, &HARD].into_iter().find(|s| s.name == name)
    }

    pub(super) fn probes(&self) -> impl Iterator<Item = &'static Probe> {
        self.turns
            .iter()
            .flat_map(|t| t.probes.iter())
            .chain(self.behavior.iter())
    }

    /// The zone of the turn that states `probe_id`; behavior probes belong to no turn and read as
    /// early.
    pub(super) fn zone_of(&self, probe_id: &str) -> Zone {
        self.turns
            .iter()
            .find(|t| t.probes.iter().any(|p| p.id == probe_id))
            .map_or(Zone::Plant, |t| t.zone)
    }
}
