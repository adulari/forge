//! `forge bench recall`: how much of a long session's early context survives compaction and a
//! resume.
//!
//! A scripted multi-turn session (planted constraints, a correction, decisions, file and
//! function names, an error and its fix, pending tasks, numbers) runs against a real model in a
//! throwaway workspace, store and config. It is compacted twice and resumed from the store;
//! after each of those, one probe per planted fact is asked as an ordinary user turn and scored
//! by regex. The scenario is data (`scenario.rs`), so a new fact is one table entry.

use std::time::Instant;

use anyhow::{Context, Result};

mod driver;
mod fixture;
mod live;
mod render;
mod scenario;
mod score;

use driver::Subject;
use live::{IsolatedEnv, LiveSubject, Price};
use scenario::Scenario;
use score::{score_phase, CompactionStats, Phase, Report};

pub(crate) const DEFAULT_MODEL: &str = "inferhub::glm-5.3-flash";

pub(crate) struct RecallOptions {
    /// `standard` or `hard`.
    pub scenario: String,
    pub model: Option<String>,
    /// Auto-compaction ceiling in tokens for the pinned model; overrides the scenario's own.
    pub compact_cap: Option<u64>,
    /// Offline deterministic provider: exercises the plumbing, recall is meaningless.
    pub mock: bool,
    pub turns_scale: f64,
    pub skip_resume: bool,
    pub keep_workdir: bool,
    pub json: bool,
    pub price_in_per_mtok: Option<f64>,
    pub price_out_per_mtok: Option<f64>,
}

pub(crate) async fn run_recall(opts: RecallOptions) -> Result<()> {
    let scenario = Scenario::by_name(&opts.scenario)
        .with_context(|| format!("unknown scenario '{}' (standard or hard)", opts.scenario))?;
    let compact_cap = opts.compact_cap.or(scenario.compact_cap).filter(|c| *c > 0);
    let model = (!opts.mock).then(|| opts.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into()));
    let price = match (opts.price_in_per_mtok, opts.price_out_per_mtok) {
        (Some(input_per_mtok), Some(output_per_mtok)) => Some(Price {
            input_per_mtok,
            output_per_mtok,
        }),
        (None, None) => None,
        _ => anyhow::bail!("--price-in and --price-out go together"),
    };
    anyhow::ensure!(
        opts.turns_scale.is_finite() && opts.turns_scale >= 0.0,
        "--turns-scale must be a non-negative number"
    );

    let started = Instant::now();
    let mut env = IsolatedEnv::enter(model.as_deref(), price, compact_cap)?;
    let workspace = tempfile::Builder::new()
        .prefix("forge-recall-ws-")
        .tempdir()
        .context("creating the bench workspace")?;
    let workdir = std::fs::canonicalize(workspace.path())?;
    fixture::create(&workdir, scenario.fixture)?;

    let mut subject = LiveSubject::start(&workdir, model.clone(), opts.mock).await?;
    let outcome = driver::run(&mut subject, scenario, opts.turns_scale, opts.skip_resume).await?;

    let log = subject.compaction_log();
    let kept = opts.keep_workdir.then(|| workspace.keep());
    let report = Report {
        scenario: scenario.name.to_string(),
        compact_cap: compact_cap.filter(|_| !opts.mock),
        model: model.unwrap_or_else(|| "mock".into()),
        turns_scale: opts.turns_scale,
        scenario_turns: outcome.scenario_turns,
        failed_turns: outcome.failed_turns,
        compactions: outcome.compactions,
        compaction_stats: CompactionStats::from_log(&log, outcome.scenario_events),
        compaction_events: log.events,
        resumed: outcome.resumed,
        after_compaction: score_phase(&outcome.probes, Phase::AfterCompaction),
        after_resume: outcome
            .resumed
            .then(|| score_phase(&outcome.probes, Phase::AfterResume)),
        usage: subject.usage(),
        wall_secs: started.elapsed().as_secs_f64(),
        session_id: subject.session_id(),
        workdir: kept.map(|p| p.display().to_string()),
        probes: outcome.probes,
    };
    if opts.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render::table(&report));
    }
    if let Some(dir) = &report.workdir {
        eprintln!("workspace kept at {dir}");
        if let Some(env_dir) = env.keep() {
            eprintln!("store (forge.db) and config kept at {}", env_dir.display());
        }
    }
    Ok(())
}
