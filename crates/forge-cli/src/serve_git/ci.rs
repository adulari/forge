//! `GET /api/sessions/{id}/ci` and `POST /api/sessions/{id}/ci/run` - the repository's workflows as
//! a local plan (docs/features/ci-parity.md), for the phone and desktop session views. The routes
//! use the same engine as `forge ci` and `/ci`; the run is the fail-fast plan, off the async
//! runtime, and its report is the one `forge ci status` shows.

use super::*;
use crate::ci_run::{make_plan, run_plan, view_for, CiView, RunOptions};

#[derive(serde::Serialize)]
struct CiResponse {
    #[serde(flatten)]
    view: CiView,
    /// `auto` or `off` (`[ci] gate`): whether goals and pushes wait on this plan.
    gate: &'static str,
    /// Set after a run: every step passed (or was not installed here).
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<bool>,
    /// The step in flight while a run is going on this worktree (a run this client may not have
    /// started, or whose request it gave up on).
    #[serde(skip_serializing_if = "Option::is_none")]
    running: Option<crate::remote::ci::SnapCiRunning>,
}

fn respond(root: PathBuf, run: bool) -> Result<CiResponse, String> {
    let (plan, settings) = make_plan(&root);
    let report = if run {
        Some(run_plan(
            &root,
            &plan,
            &settings,
            &RunOptions::default(),
            &mut |_| {},
        )?)
    } else {
        forge_config::ci_parity::report::load_report(&root)
    };
    Ok(CiResponse {
        view: view_for(&plan, report.as_ref()),
        gate: match settings.gate {
            forge_config::ci_parity::settings::GateMode::Auto => "auto",
            forge_config::ci_parity::settings::GateMode::Off => "off",
        },
        ok: run.then(|| report.as_ref().is_some_and(|r| r.ok())),
        running: crate::ci_run::live::for_cwd(&root).and_then(|ci| ci.running),
    })
}

async fn answer(state: Arc<DaemonState>, id: String, run: bool) -> Response {
    let root = match session_repo_root(&state, &id).await {
        Ok(root) => root,
        Err((status, message)) => return err_response(status, &message),
    };
    reply(tokio::task::spawn_blocking(move || respond(root, run)).await)
}

/// The only refusal `respond` has is a run already in flight on this worktree: 409, not a crash.
fn reply(outcome: Result<Result<CiResponse, String>, tokio::task::JoinError>) -> Response {
    match outcome {
        Ok(Ok(body)) => json_response(&body),
        Ok(Err(message)) => err_response(axum::http::StatusCode::CONFLICT, &message),
        Err(_) => err_response(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "the CI plan crashed",
        ),
    }
}

pub(super) fn routes(base: &str) -> axum::Router<Arc<DaemonState>> {
    axum::Router::new()
        .route(&format!("{base}/api/sessions/{{id}}/ci"), get(session_ci))
        .route(
            &format!("{base}/api/sessions/{{id}}/ci/run"),
            post(session_ci_run),
        )
}

async fn session_ci(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    answer(state, id, false).await
}

async fn session_ci_run(
    State(state): State<Arc<DaemonState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    answer(state, id, true).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_and_its_last_run_answer_as_rows() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(
            wf.join("ci.yml"),
            "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: ok\n        run: echo fine\n  w:\n    runs-on: windows-latest\n    steps:\n      - run: dir\n",
        )
        .unwrap();
        let before = respond(dir.path().to_path_buf(), false).unwrap();
        assert_eq!(before.view.rows.len(), 2);
        assert!(before.view.summary.is_none() && before.ok.is_none() && before.running.is_none());
        let after = respond(dir.path().to_path_buf(), true).unwrap();
        assert_eq!(after.ok, Some(true));
        let json = serde_json::to_value(&after).unwrap();
        assert_eq!(json["rows"][0]["state"], "pass");
        assert_eq!(json["rows"][1]["state"], "skip");
        assert_eq!(json["gate"], "auto");
    }

    #[test]
    fn a_run_on_a_worktree_that_is_running_answers_409_with_the_step() {
        let dir = tempfile::tempdir().unwrap();
        let wf = dir.path().join(".github/workflows");
        std::fs::create_dir_all(&wf).unwrap();
        std::fs::write(
            wf.join("ci.yml"),
            "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - name: lint\n        run: echo fine\n",
        )
        .unwrap();
        let _held = crate::ci_run::live::acquire(dir.path()).unwrap();
        crate::ci_run::live::begin(dir.path(), 1, 0);
        crate::ci_run::live::step_started(dir.path(), "lint", 1, 1);
        let refused = respond(dir.path().to_path_buf(), true).err().unwrap();
        assert_eq!(
            refused,
            "a local CI run is already in progress on this worktree (step 1/1: lint)"
        );
        assert_eq!(
            reply(Ok(Err(refused))).status(),
            axum::http::StatusCode::CONFLICT
        );
    }
}
