//! CI doc-sync guard for docs/features/mesh-routing.md — the normative mesh-routing reference.
//!
//! The doc promises to reproduce every routing constant 1:1 with the code. This test enforces
//! that promise mechanically: it reads the doc at test time and asserts that, for each LIVE
//! constant (pulled from the real symbol, never a literal duplicated here), the doc states the
//! current value on a line that names the constant. Change a constant without updating the doc
//! and this test fails, naming the constant, its new value, and the doc path.

use std::collections::HashMap;

use forge_types::TaskTier;

use crate::capability;
use crate::catalog;
use crate::pricing;

const DOC_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/features/mesh-routing.md"
);

/// Render an `f64` the way the doc writes constants: integral values keep one decimal ("1.0",
/// "20.0"), fractional values print minimally ("0.15", "2.5").
fn fmt_value(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

/// Whether `line` contains `value` as a standalone numeric token — not as a substring of a
/// longer number. "0.15" must not be satisfied by "0.156" or "10.15": the adjacent character on
/// either side may not be a digit, nor a '.' that itself glues on to another digit.
fn contains_number_token(line: &str, value: &str) -> bool {
    let vlen = value.len();
    let mut start = 0;
    while let Some(rel) = line[start..].find(value) {
        let i = start + rel;
        let before_ok = match line[..i].chars().next_back() {
            None => true,
            Some(c) if c.is_ascii_digit() => false,
            Some('.') => !line[..i]
                .trim_end_matches('.')
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_digit()),
            Some(_) => true,
        };
        let after = &line[i + vlen..];
        let after_ok = match after.chars().next() {
            None => true,
            Some(c) if c.is_ascii_digit() => false,
            Some('.') => !after[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit()),
            Some(_) => true,
        };
        if before_ok && after_ok {
            return true;
        }
        start = i + vlen;
    }
    false
}

/// Assert the doc states `value` on some line that mentions `name`. The failure message names
/// the constant, its live value, and the doc path — everything needed to fix the drift.
fn assert_doc_states(doc: &str, name: &str, value: f64) {
    let rendered = fmt_value(value);
    let found = doc
        .lines()
        .any(|l| l.contains(name) && contains_number_token(l, &rendered));
    assert!(
        found,
        "doc out of sync with code: `{name}` is {rendered} in the live code, but no line \
         mentioning `{name}` in {DOC_PATH} states that value. Update the doc (and its worked \
         examples if affected)."
    );
}

#[test]
fn mesh_routing_doc_matches_live_constants() {
    let doc = std::fs::read_to_string(DOC_PATH).unwrap_or_else(|e| {
        panic!("cannot read {DOC_PATH}: {e} — the normative mesh-routing reference must exist")
    });

    // Burn-penalty tier scaling (catalog.rs).
    assert_doc_states(&doc, "BURN_K_TRIVIAL", catalog::BURN_K_TRIVIAL);
    assert_doc_states(&doc, "BURN_K_STANDARD", catalog::BURN_K_STANDARD);
    assert_doc_states(&doc, "BURN_K_COMPLEX", catalog::BURN_K_COMPLEX);

    // Bench-index → quality scale mapping (capability.rs).
    assert_doc_states(&doc, "BENCH_INDEX_DIVISOR", capability::BENCH_INDEX_DIVISOR);

    // Conservation demotion (catalog.rs).
    assert_doc_states(&doc, "CONSERVE_PENALTY", catalog::CONSERVE_PENALTY);

    // OAuth-supersedes-bridge demotion (catalog.rs).
    assert_doc_states(
        &doc,
        "BRIDGE_SUPERSEDE_PENALTY",
        catalog::BRIDGE_SUPERSEDE_PENALTY,
    );

    // Nominal token mix behind `estimated_cost` (pricing.rs) — integers, rendered bare.
    for (name, value) in [
        ("NOMINAL_INPUT_TOKENS", pricing::NOMINAL_INPUT_TOKENS),
        ("NOMINAL_OUTPUT_TOKENS", pricing::NOMINAL_OUTPUT_TOKENS),
    ] {
        let rendered = value.to_string();
        let found = doc
            .lines()
            .any(|l| l.contains(name) && contains_number_token(l, &rendered));
        assert!(
            found,
            "doc out of sync with code: `{name}` is {rendered} in the live code, but no line \
             mentioning `{name}` in {DOC_PATH} states that value."
        );
    }

    // The full cost_pref table: every (tier, cost-class) cell must appear on the doc line for
    // that tier. Cells are read from the live function, never duplicated as literals here.
    for (tier, label) in [
        (TaskTier::Trivial, "Trivial"),
        (TaskTier::Standard, "Standard"),
        (TaskTier::Complex, "Complex"),
    ] {
        for class in 0u8..=2 {
            let v = catalog::cost_pref(tier, class);
            let rendered = fmt_value(v);
            let found = doc.lines().any(|l| {
                l.contains(label) && l.contains("cost_pref") && contains_number_token(l, &rendered)
            });
            assert!(
                found,
                "doc out of sync with code: `cost_pref({label}, class {class})` is {rendered} in \
                 the live code, but no cost_pref table line for `{label}` in {DOC_PATH} states \
                 that value."
            );
        }
    }

    // Every bundled burn weight (capability.rs::known_burn_weight), resolved through the live
    // table via a representative catalog id per family.
    let families: &[(&str, &str)] = &[
        ("codex-oauth::gpt-5.6-sol", "Sol"),
        ("codex-oauth::gpt-5.6-terra", "Terra"),
        ("codex-oauth::gpt-5.6-luna", "Luna"),
        ("anthropic::claude-fable-5", "Fable"),
        ("anthropic::claude-mythos-5", "Mythos"),
        ("claude-cli::opus", "Opus"),
        ("claude-cli::sonnet", "Sonnet"),
        ("claude-cli::haiku", "Haiku"),
    ];
    for (id, label) in families {
        let w = capability::known_burn_weight(id).unwrap_or_else(|| {
            panic!("`known_burn_weight` no longer knows {id} — update the doc's burn-weight table in {DOC_PATH} and this test's family list")
        });
        let rendered = fmt_value(w);
        let found = doc.lines().any(|l| {
            l.contains(label)
                && (l.contains("burn") || l.contains("weight"))
                && contains_number_token(l, &rendered)
        });
        assert!(
            found,
            "doc out of sync with code: burn weight for `{label}` ({id}) is {rendered} in the \
             live `known_burn_weight` table, but no burn-weight line for `{label}` in {DOC_PATH} \
             states that value."
        );
    }

    // The config-override path must keep matching on the BARE model name (the documented
    // asymmetry): if this lookup semantics changes, the doc's "sharp edges" section is stale.
    let overrides: HashMap<String, f64> = [("gpt-5.6-sol".to_string(), 9.0)].into();
    assert_eq!(
        capability::subscription_burn_weight("codex-oauth::gpt-5.6-sol", &overrides),
        9.0,
        "`[mesh.burn_weights]` overrides no longer key on the bare model name — update the \
         'Config override asymmetry' section of {DOC_PATH}"
    );
}

#[cfg(test)]
mod token_matcher {
    use super::contains_number_token;

    #[test]
    fn value_must_be_a_standalone_number_token() {
        assert!(contains_number_token(
            "BURN_K_COMPLEX = 0.15 scales",
            "0.15"
        ));
        assert!(contains_number_token(
            "| Complex | 0.4 | 0.8 | 0.0 |",
            "0.8"
        ));
        assert!(contains_number_token(
            "ends the sentence with 0.15.",
            "0.15"
        ));
        // Substrings of longer numbers must NOT count.
        assert!(!contains_number_token("a value of 0.156 here", "0.15"));
        assert!(!contains_number_token("version 10.15 of macOS", "0.15"));
        assert!(!contains_number_token("pi is 3.0.15-ish", "0.15"));
    }
}
