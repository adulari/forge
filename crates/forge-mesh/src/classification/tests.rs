use super::*;

/// Ground-truth-labeled corpus for classifier accuracy — the "prove it" mechanism for a real,
/// live-reported failure: `score_prompt("produce a step-by-step plan to improve forge's mesh
/// task-classification system")` scored 0 (Trivial) despite being an obviously Complex,
/// self-referential planning task. Every entry is a realistic prompt shape actually seen or
/// plausible in normal usage — not synthetic keyword-stuffing — labeled by what the task
/// genuinely REQUIRES (per the classifier's own stated design principle), not by length or
/// surface phrasing. `classifier_accuracy_meets_bar` asserts against this corpus directly, so
/// it's both the regression guard and the numeric proof of any future change here.
const LABELED_CORPUS: &[(&str, TaskTier)] = &[
    // --- Trivial: mechanical, single-file, no real decision-making ---
    ("fix this typo", TaskTier::Trivial),
    ("rename this variable to snake_case", TaskTier::Trivial),
    ("bump the version to 1.2.3", TaskTier::Trivial),
    ("add a comment explaining this function", TaskTier::Trivial),
    ("remove this unused import", TaskTier::Trivial),
    (
        "what does this error mean: undefined variable x",
        TaskTier::Trivial,
    ),
    ("say hi", TaskTier::Trivial),
    ("what's 2+2", TaskTier::Trivial),
    ("format this file with prettier", TaskTier::Trivial),
    ("delete this commented-out line", TaskTier::Trivial),
    ("reformat this JSON file", TaskTier::Trivial),
    ("what does HTTP 429 mean", TaskTier::Trivial),
    ("explain what HTTP 429 means", TaskTier::Trivial),
    // --- Standard: real but bounded, single-concern changes ---
    (
        "add a retry-with-backoff wrapper around the HTTP client",
        TaskTier::Standard,
    ),
    (
        "write a unit test for the parse_config function",
        TaskTier::Standard,
    ),
    (
        "add input validation to the signup form",
        TaskTier::Standard,
    ),
    (
        "implement pagination for the /users endpoint",
        TaskTier::Standard,
    ),
    (
        "review the authentication flow for obvious issues",
        TaskTier::Standard,
    ),
    ("compare these two sorting approaches", TaskTier::Standard),
    (
        "check the performance of this endpoint under load",
        TaskTier::Standard,
    ),
    (
        "add a CLI flag to skip the confirmation prompt",
        TaskTier::Standard,
    ),
    (
        "write a script that renames all .jpeg files to .jpg",
        TaskTier::Standard,
    ),
    (
        "port this Python script to a bash script",
        TaskTier::Standard,
    ),
    // --- Complex: real design/reasoning/architectural stakes ---
    ("investigate why the cache warms slowly", TaskTier::Complex),
    (
        "audit the permission checks in the auth module",
        TaskTier::Complex,
    ),
    (
        "debug the race condition in the scheduler",
        TaskTier::Complex,
    ),
    (
        "design a plan to migrate the database to Postgres",
        TaskTier::Complex,
    ),
    ("architect a plugin system for the CLI", TaskTier::Complex),
    (
        "there is a memory leak in the connection pool, find it",
        TaskTier::Complex,
    ),
    // The exact reported failure — hyphenated "step-by-step", no other strong keyword.
    (
        "produce a step-by-step plan to improve forge's mesh task-classification system",
        TaskTier::Complex,
    ),
    (
        "produce a step-by-step plan to improve the auth module",
        TaskTier::Complex,
    ),
    (
        "come up with a plan for refactoring the billing service",
        TaskTier::Complex,
    ),
    (
        "propose an approach for making the API idempotent",
        TaskTier::Complex,
    ),
    (
        "what's the best way to restructure this module — think it through",
        TaskTier::Complex,
    ),
    (
        "evaluate whether we should switch to a different ORM",
        TaskTier::Complex,
    ),
    (
        "think hard about the tradeoffs here before answering",
        TaskTier::Complex,
    ),
    (
        "give me an in-depth review of this design",
        TaskTier::Complex,
    ),
    (
        "investigate then fix the flaky test, explaining the root cause",
        TaskTier::Complex,
    ),
    (
        "re-evaluate our current difficulty tiers and check if this is the best setup",
        TaskTier::Complex,
    ),
    (
        "dig into why the mesh keeps under-routing tasks and fix it, proven with real testing",
        TaskTier::Complex,
    ),
];

#[test]
fn classifier_accuracy_meets_bar() {
    let mut failures = Vec::new();
    for (prompt, expected) in LABELED_CORPUS {
        let got = score_prompt(prompt, &ProjectContext::default()).tier;
        if got != *expected {
            failures.push(format!("{prompt:?}: expected {expected:?}, got {got:?}"));
        }
    }
    let accuracy = 1.0 - (failures.len() as f64 / LABELED_CORPUS.len() as f64);
    assert!(
        failures.is_empty(),
        "classifier accuracy {:.1}% ({}/{} correct) — failures:\n{}",
        accuracy * 100.0,
        LABELED_CORPUS.len() - failures.len(),
        LABELED_CORPUS.len(),
        failures.join("\n")
    );
}

#[test]
fn self_hosting_escalates_infra_talk_that_would_otherwise_be_trivial() {
    // No REASONING_TERMS/ACTION_VERBS/ANALYSIS_TERMS hit here — outside a self-hosting
    // session this scores 0 (Trivial). The self-hosting signal is the ONLY thing that
    // should change the verdict, proving it actually does something rather than being
    // decorative — the same words in an unrelated project must NOT get the bump.
    let p = "look at the mesh routing code";
    assert_eq!(
        score_prompt(p, &ProjectContext::default()).tier,
        TaskTier::Trivial,
        "outside self-hosting, infra vocabulary alone must not escalate"
    );
    let self_hosting = ProjectContext {
        project_name: Some("forge-agent".to_string()),
        is_self_hosting: true,
    };
    assert_eq!(
        score_prompt(p, &self_hosting).tier,
        TaskTier::Complex,
        "self-hosting must escalate the SAME prompt"
    );
}

#[test]
fn self_hosting_does_not_escalate_unrelated_infra_talk_in_a_different_project() {
    // A project that happens to use the words "mesh"/"router" for its OWN unrelated purpose
    // (is_self_hosting: false) must not get the bump just because those words appear.
    let unrelated = ProjectContext {
        project_name: Some("some-other-app".to_string()),
        is_self_hosting: false,
    };
    assert_eq!(
        score_prompt("look at the mesh routing code", &unrelated).tier,
        TaskTier::Trivial
    );
}
