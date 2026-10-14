//! Gateway ids against the real feed. Every id below is verbatim from InferHub's `/models`
//! (snapshot 2026-10-14) and every row verbatim from the Artificial Analysis cache of the same day,
//! intelligence and coding indices included. Synthetic fixtures are how the version-guard bugs got
//! past review twice: a made-up feed rarely carries both "Opus 5" and "Opus 5.5".

use super::*;

fn feed() -> BenchmarkScores {
    let mut b = BenchmarkScores::new();
    for (name, intelligence, coding) in [
        ("Claude Opus 5.5 (Max, Default Fallback)", 57.6, 81.6),
        ("Claude Opus 5.5 (Xhigh, Default Fallback)", 56.0, 81.0),
        ("Claude Opus 5.5 (High, Default Fallback)", 53.6, 80.0),
        ("Claude Opus 5.5 (Low, Default Fallback)", 42.3, 75.0),
        ("Claude Sonnet 5.5 (Max, Default Fallback)", 56.0, 79.0),
        ("Claude Opus 5 (Max)", 50.8, 78.0),
        ("Claude Opus 5 (High)", 48.1, 76.5),
        ("Claude Opus 5 (Low)", 39.4, 66.9),
        ("Claude Sonnet 5 (Max)", 38.2, 71.5),
        ("Claude Sonnet 5 (Non-reasoning, High)", 23.2, 66.4),
        ("Claude Fable 5 (Max, Opus 4.8 Fallback)", 49.6, 76.5),
        ("Claude Opus 4.8 (Max)", 41.8, 74.3),
        ("Claude Opus 4.7 (Max)", 40.7, 73.6),
        ("DeepSeek V4.1 Flash (Max)", 39.5, 70.0),
        ("DeepSeek V4 Flash 0731 (Max)", 34.3, 69.1),
        ("DeepSeek V4 Flash 0420 (Max)", 24.2, 56.2),
        ("DeepSeek V4 Flash Vision (Max)", 34.8, 65.0),
        ("Qwen3.8 Max (0902)", 45.4, 76.2),
        ("Qwen3.8 Max", 40.2, 71.8),
        ("Qwen3.8-Flash-Next", 39.8, 73.1),
        ("GLM-5.3 (Max)", 44.8, 74.8),
        ("GLM-5.3-Flash", 41.8, 71.5),
        ("GLM-5 (Reasoning)", 27.9, 50.0),
        ("Gemini 3.8 Flash (High)", 40.9, 76.3),
        ("Gemini 3.8 Flash (Low)", 33.5, 73.5),
        ("Gemini 3 Pro Preview (High)", 28.0, 60.0),
        ("Gemini 1.0 Pro", 5.3, 5.0),
        ("Ling 3.1 Flash", 41.1, 60.0),
        ("Ling-mini-2.0", 5.5, 5.0),
        ("Qwen2.5 Turbo", 6.4, 6.0),
        ("GPT-5.6 Sol (Max)", 47.0, 77.4),
        ("GPT-5.6 Terra (Max)", 42.1, 76.7),
        ("GPT-5.6 Luna (Max)", 37.3, 71.4),
        ("Kimi K3 (Max)", 43.6, 76.2),
        ("Kimi K2.7 Code", 25.8, 60.8),
        ("Muse Spark 1.3 (Max)", 48.1, 75.8),
        ("Muse Spark 1.2 (Xhigh)", 39.6, 72.2),
        ("Muse Spark", 31.3, 58.6),
    ] {
        b.insert(name, intelligence, coding);
    }
    b
}

#[test]
fn every_inferhub_id_resolves_to_its_own_row_or_to_none() {
    let b = feed();
    // (id, intelligence of the row it must take — None = must stay unscored)
    let table: &[(&str, Option<f64>)] = &[
        // The reported bug: a shorter version zipped against a point release found no conflict.
        ("inferhub::kiro-opus-5", Some(50.8)),
        ("inferhub::kiro-sonnet-5", Some(38.2)),
        ("inferhub::kiro-opus-5.5", Some(57.6)),
        ("inferhub::kiro/claude-opus-5", Some(50.8)),
        ("inferhub::cc/claude-opus-5-5", Some(57.6)),
        ("inferhub::ag/claude-sonnet-5-5", Some(56.0)),
        ("inferhub::cc/claude-fable-5", Some(49.6)),
        ("inferhub::kiro-opus-4.8", Some(41.8)),
        ("inferhub::cb/claude-opus-4.7-1m", Some(40.7)),
        ("inferhub::kiro-glm-5", Some(27.9)),
        // Undated V4 Flash is the newest V4 snapshot, never V4.1 and never the Vision variant.
        ("inferhub::deepseek-v4-flash", Some(34.3)),
        ("inferhub::cmc/deepseek/deepseek-v4-flash", Some(34.3)),
        ("inferhub::ali/deepseek-v4-flash-0731", Some(34.3)),
        ("inferhub::deepseek-v4.1-flash-fast", Some(39.5)),
        // A snapshot parenthetical is a different model, not decoration.
        ("inferhub::qwen3.8-max", Some(40.2)),
        ("inferhub::ali/qwen3.8-max-0902", Some(45.4)),
        ("inferhub::cbcn/glm-5.3-flash", Some(41.8)),
        ("inferhub::zai/glm-5.3", Some(44.8)),
        ("inferhub::cmc/meta/muse-spark-1.3-contributor", Some(48.1)),
        ("inferhub::muse-spark-1.2-contributor", Some(39.6)),
        // Kimi Code plan slots resold by the gateway.
        ("inferhub::kc/k3", Some(43.6)),
        ("inferhub::kcai/k3-256k", Some(43.6)),
        ("inferhub::kc/kimi-for-coding-highspeed", Some(25.8)),
        ("inferhub::cmc/moonshotai/Kimi-K3", Some(43.6)),
        // A tier the feed has no row for is unscored, not its sibling.
        ("inferhub::qwen3.8-flash", None),
        ("inferhub::qwen3.8-omni-flash", None),
        ("inferhub::glm-5.3-flashx", None),
        ("inferhub::gemini-omni-flash", None),
        ("inferhub::mm/MiniMax-M3.1-Flash-Preview", None),
        // An id that names no sibling cannot borrow the best one.
        ("inferhub::kiro-gpt-5.6", None),
        ("inferhub::kiro-gpt-5.6-terra", Some(42.1)),
        // Media models share only tier words with chat rows.
        ("inferhub::seedance-1.0-pro", None),
        ("inferhub::seedance-2.0-mini", None),
        ("inferhub::krea-2-turbo", None),
        ("inferhub::cb/gemini-3.0-pro-image", None),
    ];
    for (id, want) in table {
        assert_eq!(
            b.score_for(id).map(|s| s.intelligence),
            *want,
            "{id} matched the wrong row"
        );
    }
}

#[test]
fn an_id_that_names_its_rung_scores_at_that_rung_and_has_no_ladder() {
    let b = feed();
    let low = b.score_for("agy-cli::gemini-3.8-flash-low").expect("rated");
    assert_eq!(
        low.intelligence, 33.5,
        "the low variant is not the high row"
    );
    let high = b
        .score_for("inferhub::ag/gemini-3.8-flash-high")
        .expect("rated");
    assert_eq!(high.intelligence, 40.9);
    assert!(
        b.efforts_for("inferhub::ag/gemini-3.8-flash-high")
            .is_empty(),
        "a model pinned to a rung by its name has no ladder to route along"
    );
    assert_eq!(
        b.efforts_for("inferhub::gemini-3.8-flash")
            .iter()
            .map(|(e, _)| *e)
            .collect::<Vec<_>>(),
        [BenchEffort::Low, BenchEffort::High],
        "the unsuffixed id keeps the full ladder"
    );
}

#[test]
fn the_current_anthropic_spelling_keeps_its_ladder() {
    // "(Max, Default Fallback)" dropped the "Effort" word the parser looked for, which silently
    // emptied every Claude 5.5 ladder — mesh then scored Opus 5.5 at Max whatever rung it sent.
    let b = feed();
    assert_eq!(
        b.efforts_for("inferhub::cc/claude-opus-5-5")
            .iter()
            .map(|(e, _)| *e)
            .collect::<Vec<_>>(),
        [
            BenchEffort::Low,
            BenchEffort::High,
            BenchEffort::XHigh,
            BenchEffort::Max
        ]
    );
    assert_eq!(
        BenchEffort::from_source_name("Quasar 438B (Max, Based on GLM-5.2)"),
        None,
        "a lineage note still disqualifies a bare rung"
    );
}

#[test]
fn a_bridge_serving_another_vendor_is_not_given_its_own_family() {
    let mut b = feed();
    b.insert("Claude Opus 5.5 (Medium, Default Fallback)", 51.2, 79.0);
    let medium = b
        .score_for("agy-cli::claude-opus-5-5-medium")
        .expect("agy serves Claude too");
    assert_eq!(medium.intelligence, 51.2);
}

#[test]
fn a_version_less_family_row_is_the_first_release_only() {
    let b = feed();
    assert!(
        b.source_score_for("opencode::muse-spark-1.4").is_none(),
        "\"Muse Spark\" is 1.0 — it must not stand in for an unrated 1.4"
    );
    assert_eq!(
        b.score_for("meta::muse-spark-1.0").unwrap().intelligence,
        31.3
    );
}
