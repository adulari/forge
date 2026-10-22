//! The identifier-migration prompts keep their bounded sweep and the alias rule for a deprecation,
//! but a rename is not a licence to add aliases or to edit a document that freezes a name.

use crate::completeness::{
    DIRECT_COMPLETENESS_PROMPT, DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT,
    DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT, DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE,
};

const FIRST_PASS_AND_AUDIT: [&str; 2] = [
    DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE,
    DIRECT_COMPLETENESS_PROMPT,
];

const EVERY_MIGRATION_PROMPT: [&str; 4] = [
    DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE,
    DIRECT_COMPLETENESS_PROMPT,
    DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT,
    DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT,
];

#[test]
fn the_sweep_that_won_the_docs_drift_task_is_unchanged() {
    let guidance = DIRECT_IDENTIFIER_MIGRATION_SCOPE_GUIDANCE;
    for kept in [
        "ONE bounded production-scope sweep",
        "EACH old/deprecated identifier as a plain literal",
        "maximum TWO search commands",
        "Search snippets are not inspection",
        "Keep the sweep bounded",
    ] {
        assert!(guidance.contains(kept), "first-pass guidance lost: {kept}");
    }
    for kept in [
        "maximum THREE",
        "snippets are NOT inspection",
        "plain literal",
        "CONCRETE OMISSION",
    ] {
        assert!(
            DIRECT_COMPLETENESS_PROMPT.contains(kept),
            "audit prompt lost: {kept}"
        );
    }
}

#[test]
fn a_deprecation_still_requires_the_old_name_as_a_fallback() {
    for prompt in FIRST_PASS_AND_AUDIT {
        assert!(prompt.contains("For a deprecation, any"), "{prompt}");
        assert!(prompt.contains("CONCRETE OMISSION"), "{prompt}");
        assert!(prompt.contains("retaining the old alias"), "{prompt}");
    }
}

#[test]
fn a_plain_rename_does_not_require_an_alias() {
    for prompt in FIRST_PASS_AND_AUDIT {
        assert!(
            prompt.contains("A plain rename needs no alias unless the task asks for compatibility"),
            "{prompt}"
        );
        assert!(
            !prompt.contains("For a deprecation or rename, any"),
            "the alias rule must not cover a rename: {prompt}"
        );
    }
    assert!(DIRECT_COMPLETENESS_UNHANDLED_PATH_PROMPT.contains("for a plain rename"));
    assert!(DIRECT_COMPLETENESS_UNRESOLVED_RETRY_PROMPT.contains("for a plain rename"));
}

#[test]
fn every_migration_prompt_exempts_a_name_the_repo_freezes() {
    for prompt in EVERY_MIGRATION_PROMPT {
        assert!(
            prompt.contains("declares frozen or public")
                && prompt.contains("is NOT an omission")
                && prompt.contains("never edit the file that declares it"),
            "{prompt}"
        );
    }
}
