//! Phrasing tables for the read-only classifier, in both directions. Every prompt is worded for this
//! file; the shapes (protected-path lists, narrowing tails, bullets, negated verbs, mixed requests)
//! are the ones real change requests carry.

use super::explicitly_read_only;
use crate::turn_contract::{ContractSource, TurnContract};
use crate::TaskIntent;
use forge_types::PermissionMode;

/// A change request with a constraint, a description of something read-only, or a list of protected
/// paths: none of these may deny the turn its writes.
const NOT_READ_ONLY: &[&str] = &[
    // Protected-path lists next to a request to fix and run something.
    "Our billing sync keeps failing on the partner file that arrived this morning. The error is in `logs/sync.err` and the file is `incoming/p-0614.tsv`. Track down the root cause and repair it so every line gets imported; hiding the failure behind a catch-all or skipping bad lines is not acceptable, and the partner file stays untouched. Once it works, run the sync for that file. Do not modify anything under `logs/`, `jobs/` or `incoming/`, nor `NOTES.md`, `Makefile`, the test suite, or `billing/runner.py`, `billing/cli.py` and `billing/main.py`.",
    "Fix the flaky scheduler test. Don't touch anything in tests/fixtures.",
    "Implement the retry decorator. Do not change anything outside `src/retry/`.",
    "Please debug the crash in the importer. Do not edit any files except `importer.py`.",
    "Fix the off-by-one in pagination. Do not touch the generated client or the vendored copy.",
    "Fix the broken date parser.\n\nConstraints:\n- Do not modify README.md\n- Do not modify setup.py\n- Do not touch anything under docs/\n- The existing tests must stay as they are",
    "Implement the feature below.\n\nProtected files:\n- Do not edit:\n  - `a.py`\n  - `b.py`\n",
    "Add a --dry-run flag to the CLI. Do not edit\n- cli/legacy.py\n- cli/compat.py",
    "Refactor the cache layer so it is thread safe. Don't make changes to anything in `core/`.",
    "Speed up the report. Do not modify any of the following files: loader.py, schema.py, cli.py.",
    "Fix the rounding bug; do not modify any file other than `money.py`.",
    "Update the changelog. Do not alter anything beyond the Unreleased section.",
    "Fix it. Don't change anything else.",
    "Fix it. Do not modify anything else in the repo.",
    "Fix the bug and make no changes to the public interface.",
    "Fix the bug without changing files under tests/.",
    "Fix the bug without touching the lockfile.",
    "Fix the flaky test without modifying anything in `src/legacy`.",
    "Fix the bug. Do not touch the tests, the docs, or the build scripts.",
    "Debug why the cron job fails and repair it. Do not edit the input spreadsheet; accounting exports it.",
    "The input is `data/in.csv`. Fix the importer. Don't edit it, it comes from finance.",
    "Fix the typo in the heading. Don't change anything except the heading.",
    "Fix it. Do not modify anything, except `CHANGELOG.md`.",
    "Fix it. Do not edit anything (except notes.md).",
    "Fix it. Do not edit anything - only ANSWER.md may change.",
    "Fix the crash. Don\u{2019}t edit anything under tests/.",
    // Negated verbs other than "do not edit".
    "Fix it. You must not touch the schema files.",
    "Fix the parser; leave the fixtures alone.",
    "Fix the parser and leave everything under vendor/ alone.",
    "Only change `src/parser.py`; everything else stays as it is.",
    "Add the missing validation. The existing tests must stay as they are.",
    "Refactor the loader. Keep the existing tests unchanged and the public API as is.",
    "Implement the feature. Never modify generated files.",
    "Fix it, but keep your hands off the CI config.",
    "Fix the build; changing the CI config is out of bounds.",
    // A ban on the code inside a request that edits something else.
    "Update the docs to match the new flag. Don't touch the code.",
    "Update the docs; do not touch the code.",
    "Fix the typo in README. Don't change the code.",
    "Add a section to the README. Don't change the code.",
    "Please update the comments in utils.py but do not touch the code.",
    "Rewrite the intro paragraph. Do not edit the code or the tests.",
    "Reword the error messages. Don't modify the project structure.",
    // Objects that restrict what may change.
    "Fix the crash. Don't edit anything unrelated to it.",
    "Fix the crash and don't change anything you don't need to.",
    "Fix the crash; don't modify anything that is not required.",
    "Make the smallest possible fix. Don't refactor, don't rename, don't reformat anything.",
    "Don't make unrelated changes. Fix the crash in the parser.",
    "Fix it. Don't make any other changes.",
    "Please don't change the behaviour of the public methods; fix the leak.",
    "Fix the leak. Do not modify the function signatures.",
    "Fix the bug but don't edit the generated code in `gen/`.",
    "Fix the bug but don't edit code.py",
    "Update files.json with the new field. Don't edit files.py.",
    "Fix the broken link. Do not edit the files listed below.\n- docs/a.md\n- docs/b.md",
    "Fix it. Do not edit these files: a.py, b.py.",
    "Fix it. Do not edit the file.",
    "Fix the issue and do not edit the project's lockfile.",
    "Fix it. Don't edit the source of the vendored library.",
    "Fix it. Don't touch the repo's CI files.",
    "Fix it. Don't edit the code in vendor/.",
    "Fix it. Don't edit the code that calls the API.",
    "Fix it. Don't change the code style.",
    "Fix the importer. Do not edit or create any files under `data/`.",
    "Fix the importer. Do not edit, move or delete anything inside `data/`.",
    "Please fix the typo and don't modify any file in tests/ or docs/.",
    "Fix it. Do not modify any files that match *.lock.",
    "Fix it. Do not touch any files named in the ticket.",
    "Fix it. Do not change any file but the parser.",
    "Fix it, and do not edit anything besides parser.py.",
    "Fix it. Do not edit anything inside the `legacy` package.",
    "Fix it. Don't change any files outside of src/.",
    "Fix it. Please do not modify the schema, the migrations, or any generated files.",
    "Fix it. Don't change any files in the repo other than the parser.",
    "Fix it. Do not modify anything in the repository except src/lib.rs.",
    "Fix the retry logic. Don't modify anything in the vendored directories, nor the build files.",
    "Fix the failing assertion; don't touch the test file.",
    "Fix the failing assertion; don't alter the test.",
    "Update the version string. Don't change anything else, not even whitespace.",
    "Add a unit test for the parser. Do not modify the parser itself.",
    "Implement pagination. Do not write any files outside `src/`.",
    "Write the new module. Do not write to any file in `legacy/`.",
    "Fix the leak without making changes to the public API.",
    "Make no changes to the tests, fix the bug.",
    "Don't change a thing in the fixtures. Fix the parser.",
    "Fix the build and do not edit anything above the marker line in `Makefile`.",
    "Fix it. Do not make changes to the generated files.",
    "Fix it. Don't make edits to anything in the data directory.",
    "Fix it. Do not make any changes outside the parser module.",
    "Fix it. Please don't write any files outside the output directory.",
    "Fix it, do not modify, rename or delete the existing tests.",
    "Fix it, don't edit, reformat or relocate the tests.",
    "Fix it. Do not edit, tweak or otherwise alter the fixtures.",
    "Fix it. Don't edit anything in `tests/` while you work.",
    // Bullet and numbered constraint sections.
    "Fix the crash.\n- Don't edit the incoming file.\n- Don't swallow errors.\n- Don't skip rows.",
    "Task: fix the parser.\nConstraints:\n- do not modify README.md\n- do not touch anything under docs/\n- keep the tests as they are",
    "Fix the build.\n\nRules:\n1. Do not modify the lockfile.\n2. Do not touch anything in `.github/`.\n3. Keep the diff small.",
    "Fix the bug.\nDo not modify\n  - the lockfile\n  - the CI config",
    // Mixed requests: something to explain, something to change, something protected.
    "Explain why the build fails, then fix it. Do not edit the lockfile.",
    "Review `src/auth.rs` and fix any bug you find. Do not touch tests.",
    "Look at the failing job and repair it. Do not change the pipeline definition.",
    "What is wrong with the date parser? Fix it, and don't change the tests.",
    "Fix the parser. Don't edit it.",
    "Do not edit anything but the docs.",
    "Implement the endpoint; make no changes to existing endpoints.",
    "Add logging without changing any behaviour.",
    "Rename the function everywhere. Don't touch anything in vendor.",
    "Update the dependency. Do not change any other dependency.",
    "Fix it. Do not edit any of the other files.",
    "Fix it. Don't modify the source.",
    "Implement the parser. Don't edit the tokenizer, nor the AST module.",
    "Fix the failing test. (Don't touch anything in src/ though.)",
    "I rotate onto the billing worker next week and need to explain it. Don't change any code. Read through it and write what you find to `NOTES.md` in the repo root.",
    "Don't change any code, only documentation: add a short troubleshooting section to the README.",
    // Read-only as an adjective on something, not an instruction about the turn.
    "Read-only files: `data/`, `logs/`. Fix the exporter.",
    "Read-only mounts: /data. Fix the loader.",
    "Read-only access to the prod DB is fine, but fix the report bug locally.",
    "This is read-only data under fixtures/, so fix the parser instead.",
    "This is a read-only filesystem in CI, so make the test write to a temp dir.",
    "Fix it. The snapshot files are read-only.",
    "The `vendor/` tree is read-only; fix the bug elsewhere.",
    "Read only copies of the inputs live in `inputs/`. Fix the importer.",
    "Fix the config loader so it handles a read-only home directory.",
    "Make the exporter work when the output dir is read-only.",
    "Add a flag that opens the file read-only.",
    "Inspect with read-only tools, then implement the fix.",
];

/// Prompts that ask for no change, in the many ways people type that.
const READ_ONLY: &[&str] = &[
    "Explain how the retry policy works. Do not edit anything.",
    "Make no edits whatsoever.",
    "Explain this code and don't touch it.",
    "Read the file and tell me what it does. Don't touch it.",
    "Do not edit anything but explain.",
    "Please investigate why startup is slow. Do not make any code changes, and do not commit.",
    "Please look at the scheduler and, without making any changes, tell me how it picks a task.",
    "Walk me through `src/router.rs`; don't change any files.",
    "Audit the auth module for injection risks. Read-only.",
    "Read-only review of the scheduler, please.",
    "Constraints:\n- Do not edit anything.\n- Answer in under 200 words.",
    "Look at the failing test and tell me why it fails, but don't change any files.",
    "Review the diff, do not modify anything, and list the risks.",
    "Do not make any changes. Just tell me what the nightly job does.",
    "Don't touch anything; I only want an explanation.",
    "Explain the build system without changing anything.",
    "Answer from the code as it is, without modifying any files.",
    "Please do not edit or create any files. Summarize the module layout.",
    "Tell me what is wrong with the importer. Do NOT change anything.",
    "Don't edit anything at all.",
    "Please don't modify any files whatsoever.",
    "Do not modify anything \u{2014} just report back.",
    "Do not change anything until I confirm.",
    "Don't touch any files while you look around.",
    "Do not modify anything in the repository, I only need an explanation.",
    "Do not change code or files; this is only an investigation.",
    "Don't write any files or run anything that changes state.",
    "Explain how the cache works. Don't change the code.",
    "Explain the flow. Don't edit the code.",
    "Don't edit the code, just explain the flow.",
    "Explain what `parse_args` does. Do not change anything in the repo.",
    "Describe the retry loop. Do not edit any file in the project.",
    "Summarize the open TODOs. Make no edits.",
    "Make no changes to anything; I only want a summary.",
    "List the public functions. Do not make changes to any files.",
    "Do not make changes to the code. Explain the algorithm.",
    "Review this and do not edit any files.",
    "Look, but do not make changes.",
    "Explain it - don't edit anything.",
    "Answer only \u{2014} don't edit \u{2014} then stop.",
    "Explain the parser. Don't edit.",
    "Explain it. Do not edit, delete, or rename anything.",
    "Explain it. Do not edit, delete or rename files.",
    "Explain it. Don't edit, rewrite or refactor - just explain.",
    "Explain it. Don't edit, just explain.",
    "Explain it. Don't edit files yet.",
    "Explain it. Don't edit code yet.",
    "Explain it. Don't edit it in any way.",
    "Explain it. Do not edit files in place.",
    "Look around. Do not change anything.",
    "Explain it, don't modify anything please",
    "One command, then answer. Do not edit anything.\n\nRun: rg -n 'last_relay' crates/forge-cli/src\n\nThen answer two questions.",
    "Read-only: inspect the parser",
    "read only \u{2014} tell me what the router does",
    "- **Read-only**: audit the store",
    "Note: read-only investigation, no edits",
    "Audit the store. This is read-only.",
    "Answer without changing files",
    "Do not make changes; just explain the failover chain",
    "This is a read-only question: which module owns the retry policy?",
    "Treat this as read-only and tell me where the config is parsed.",
    "Please keep this read-only. Which crate defines the router?",
    "Do not change anything in any file.",
    "Do not edit, create or delete any files. Report only.",
    "Check the logs and tell me what failed. Don't touch anything on disk.",
    "Investigate the memory growth. Make no edits, make no commits.",
    "Do not edit anything, and do not run anything that writes.",
    "Explain the module. Do not edit any of the files.",
    "Just explain. Don't change any files, and don't run migrations.",
    "Don't edit any files; read and report.",
    "Don't modify anything. Report the findings in a table.",
    "Explain this stack trace. Do not change any file in the project.",
    "Do not change any code. Explain what the function does.",
    "Task: explain the data flow.\nRules:\n- Do not edit anything.\n- Do not run the tests.",
    "Explain the lifecycle of a request. Do not touch any files.",
    "I only want an analysis. Don't alter anything.",
    "Do NOT edit anything. Do NOT run mutating commands.",
    "Read-only. Tell me what the cron job does.",
    "> Read-only: summarize the repo layout",
    "Please don't edit anything \u{2014} I'm just looking.",
    "Don't edit files, don't create files, just answer.",
    "Without editing anything, tell me how the scheduler picks a task.",
    "Tell me how the planner picks a model, without touching any file.",
];

fn misread(prompts: &[&str], expected: bool) -> Vec<String> {
    prompts
        .iter()
        .filter(|prompt| explicitly_read_only(prompt) != expected)
        .map(|prompt| format!("{prompt:?}"))
        .collect()
}

#[test]
fn no_constraint_on_a_change_request_reads_as_a_read_only_turn() {
    let wrong = misread(NOT_READ_ONLY, false);
    assert!(
        wrong.is_empty(),
        "{} of {} change requests were read as read-only:\n{}",
        wrong.len(),
        NOT_READ_ONLY.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_way_of_asking_for_no_change_is_still_read_only() {
    let wrong = misread(READ_ONLY, true);
    assert!(
        wrong.is_empty(),
        "{} of {} read-only requests were missed:\n{}",
        wrong.len(),
        READ_ONLY.len(),
        wrong.join("\n")
    );
}

/// The bench instruction that exposed this wording, restated: a bug report with a fix request, a
/// protected input, a protected tree and a list of protected files, ending in a run command.
#[test]
fn a_long_protected_file_list_does_not_deny_the_turn_its_writes() {
    let prompt = "Nightly reconciliation stopped halfway through today's bank statement (`statements/0614.ofx`, trace in `run/recon.trace`). Work out why, make it handle the statement properly, and leave no transaction out of the result. The statement is generated by the bank, so it must not be edited.\n\nThen run the reconciliation for that date by calling the module directly. Do not modify anything under `run/`, `cron/` or `statements/`, nor `CONTRIBUTING.md`, `tox.ini`, the current tests, or `recon/engine.py`, `recon/models.py`, `recon/cli.py` and `recon/__main__.py`.";
    assert!(!explicitly_read_only(prompt));
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Bypass,
    ] {
        for expects_change in [false, true] {
            let contract = TurnContract::derive(prompt, mode, expects_change);
            assert_eq!(contract.intent(), TaskIntent::Mutating, "{mode:?}");
            assert_ne!(contract.source(), ContractSource::ExplicitReadOnly);
            assert!(
                contract
                    .guidance()
                    .is_none_or(|guidance| !guidance.contains("read-only")),
                "{mode:?} {expects_change}"
            );
        }
    }
}

#[test]
fn a_read_only_request_with_protected_paths_named_stays_read_only_when_it_says_so() {
    let prompt = "Explain how `src/syncer/runner.py` reads `feeds/incoming/0614.tsv`. Do not modify anything under `src/`, and do not edit any files at all.";
    assert!(explicitly_read_only(prompt));
    let contract = TurnContract::derive(prompt, PermissionMode::Default, true);
    assert_eq!(contract.source(), ContractSource::ExplicitReadOnly);
}

/// Every protected object the prompts above use, crossed with every way of forbidding an edit and
/// every way of framing a change request. None of the combinations may be read-only.
#[test]
fn every_scoped_prohibition_in_every_frame_is_a_constraint() {
    let verbs = [
        "do not edit",
        "don't edit",
        "Do NOT modify",
        "don't change",
        "do not touch",
        "please don't alter",
        "do not make changes to",
        "don't make any changes to",
        "do not edit, delete, or rename",
        "do not modify, rename or remove",
        "do not write to",
    ];
    let objects = [
        "`logs/`",
        "anything under `logs/`",
        "anything under `logs/`, `ops/` or `data/`",
        "any file outside src/",
        "files other than ANSWER.md",
        "anything except the heading",
        "the existing tests",
        "README.md",
        "README.md, setup.py and tox.ini",
        "the files in examples/",
        "anything else",
        "any of the following files: a.py, b.py",
        "the generated code in gen/",
        "the public API",
        "tests/",
        "the lockfile",
        "the incoming file",
        "anything unrelated to the bug",
        "any other file",
        "code outside of src/lib",
        "everything under vendor/",
        "any files matching *.snap",
        "files that the tests import",
        "anything inside the legacy package",
    ];
    let frames = [
        ("Fix the crash in the loader. ", ""),
        (
            "Implement the new flag and add tests. ",
            ", it is generated.",
        ),
        ("Update the docs. ", " and keep the diff small."),
        ("Please debug the failing export and repair it. ", ""),
        ("Constraints:\n- ", "\n- keep the tests as they are"),
        ("Refactor the parser; ", "; the rest is frozen."),
    ];
    let mut wrong = Vec::new();
    for verb in verbs {
        for object in objects {
            for (lead, tail) in frames {
                let prompt = format!("{lead}{verb} {object}{tail}");
                let prompt =
                    if prompt.ends_with('.') || prompt.ends_with('\n') || tail.contains("keep") {
                        prompt
                    } else {
                        format!("{prompt}.")
                    };
                if explicitly_read_only(&prompt) {
                    wrong.push(format!("{prompt:?}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} read as read-only:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

#[test]
fn every_universal_prohibition_in_every_explanatory_frame_is_read_only() {
    let verbs = [
        "do not edit",
        "don't edit",
        "Do NOT modify",
        "don't change",
        "do not touch",
        "please don't alter",
        "do not make changes to",
        "don't make any changes to",
        "do not edit, delete, or rename",
        "do not modify, rename or remove",
    ];
    let objects = [
        "anything",
        "any files",
        "any file",
        "files",
        "anything at all",
        "any files whatsoever",
        "anything in the repo",
        "any file in the project",
        "any of the files",
        "anything on disk",
        "any code",
        "the code",
        "the repo",
    ];
    let frames = [
        ("Explain how the loader works. ", "."),
        ("Summarize the module. ", ", just explain."),
        (
            "Tell me why the test fails. ",
            " and tell me what you find.",
        ),
        ("Review this diff. ", "; read and report."),
        ("Constraints:\n- ", "\n- answer briefly"),
        ("", " \u{2014} I only want an explanation."),
    ];
    let mut wrong = Vec::new();
    for verb in verbs {
        for object in objects {
            for (lead, tail) in frames {
                let prompt = format!("{lead}{verb} {object}{tail}");
                if !explicitly_read_only(&prompt) {
                    wrong.push(format!("{prompt:?}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{} missed:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
}

/// A bare ban on the code, content or project only reads as "change nothing" when the prompt asks
/// for no change: "update the docs, don't touch the code" is a change request.
#[test]
fn a_ban_on_the_code_follows_whether_the_prompt_asks_for_a_change() {
    for object in [
        "the code",
        "code",
        "any code",
        "the codebase",
        "the repo",
        "the project",
        "the source",
    ] {
        for verb in ["do not edit", "don't touch", "do not change"] {
            let explain = format!("Explain how it is structured. {verb} {object}.");
            assert!(explicitly_read_only(&explain), "{explain}");
            for request in [
                "Update the docs",
                "Please fix the typo in the README",
                "Write the answers to ANSWER.md",
                "Add a changelog entry",
                "Rewrite the intro",
            ] {
                let change = format!("{request}. {verb} {object}.");
                assert!(!explicitly_read_only(&change), "{change}");
            }
        }
    }
}

/// The scanners slice around multi-byte characters and unusual punctuation; none of that may panic.
#[test]
fn scrambled_phrasings_never_panic() {
    let tokens = [
        "do not",
        "don't",
        "dont",
        "edit",
        "modify",
        "change",
        "touch",
        "write",
        "make",
        "no",
        "changes",
        "edits",
        "anything",
        "files",
        "file",
        "any",
        "the",
        "of",
        "to",
        "it",
        "code",
        "without",
        "making",
        "changing",
        "read-only",
        "this is",
        "yet",
        "in place",
        ",",
        ".",
        ":",
        ";",
        "\n",
        "\n- ",
        "\u{2014}",
        "(",
        ")",
        "`src/a.rs`",
        "tests/",
        "caf\u{e9}",
        "\u{65e5}\u{672c}",
        "\u{1f600}",
        " ",
        "-",
        "'",
        "\u{2019}",
        "or",
        "and",
        "but",
        "just",
        "only",
        "except",
        "else",
        "explain",
        "Fix",
        "Update",
    ];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..20_000 {
        let length = 1 + (next() % 14) as usize;
        let mut prompt = String::new();
        for _ in 0..length {
            prompt.push_str(tokens[(next() % tokens.len() as u64) as usize]);
            if next() % 3 != 0 {
                prompt.push(' ');
            }
        }
        let _ = explicitly_read_only(&prompt);
    }
}

/// Prompts with no verb in sight, and edge input, must not trip the scanners.
#[test]
fn degenerate_input_is_not_read_only() {
    for prompt in [
        "",
        " ",
        "don't",
        "do not",
        "do not edit:",
        "without",
        "make no",
        "-",
        "\u{2014}",
        "Do nothing fancy, just fix it.",
        "undo not editing",
        "I don't edit-warn",
        "Read-only files",
    ] {
        assert!(!explicitly_read_only(prompt), "{prompt:?}");
    }
    assert!(explicitly_read_only("do not edit"));
    assert!(explicitly_read_only("without editing"));
}
