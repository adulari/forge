use super::*;

fn ctx() -> PlanContext {
    PlanContext {
        runner_temp: PathBuf::from("/tmp/rt"),
        workspace: PathBuf::from("/work"),
        changed_files: None,
        branch: Some("feature/x".to_string()),
    }
}

fn plan(files: &[(&str, &str)]) -> CiPlan {
    let sources: Vec<_> = files
        .iter()
        .map(|(n, t)| (n.to_string(), Ok(t.to_string())))
        .collect();
    plan_from_sources(&sources, &ctx(), None)
}

const SCRATCH: &str = r#"
name: CI
on:
  push:
    branches: [main]
  pull_request:
env:
  STRICT: "1"
jobs:
  check:
    runs-on: ubuntu-latest
    env:
      LEVEL: ${{ runner.os }}
    steps:
      - uses: actions/checkout@v4
      - name: Strict check
        run: test "$STRICT" = 1 && ./check.sh
      - name: Deploy
        env:
          TOKEN: ${{ secrets.DEPLOY_TOKEN }}
        run: ./deploy.sh
      - name: Publish
        run: npm publish
      - name: Cleanup
        if: always()
        run: rm -rf out
  win:
    runs-on: windows-latest
    steps:
      - run: echo hi
"#;

#[test]
fn scratch_workflow_plans_the_runnable_steps_and_lists_every_skip() {
    let p = plan(&[("ci.yml", SCRATCH)]);
    assert_eq!(p.steps.len(), 1, "{:#?}", p.steps);
    let s = &p.steps[0];
    assert_eq!(s.key, "ci.yml::check::Strict check");
    assert_eq!(s.env.get("STRICT").map(String::as_str), Some("1"));
    assert_eq!(s.env.get("LEVEL").map(String::as_str), Some("Linux"));
    assert_eq!(p.noops, 1);
    let reasons: Vec<(String, String)> = p
        .skips
        .iter()
        .map(|k| (k.step.clone().or(k.job.clone()).unwrap(), k.reason.clone()))
        .collect();
    assert!(reasons.contains(&(
        "Deploy".into(),
        "env TOKEN: needs secret DEPLOY_TOKEN".into()
    )));
    assert!(reasons
        .iter()
        .any(|(n, r)| n == "Publish" && r.contains("npm publish")));
    assert!(reasons
        .iter()
        .any(|(n, r)| n == "Cleanup" && r.contains("always()")));
    assert!(reasons
        .iter()
        .any(|(n, r)| n == "win" && r.contains("windows-latest")));
}

#[test]
fn workflows_that_do_not_run_on_push_or_pr_are_skipped_with_the_reason() {
    let p = plan(&[(
        "nightly.yml",
        "on:\n  schedule:\n    - cron: '0 1 * * *'\n  workflow_dispatch:\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n",
    )]);
    assert!(p.steps.is_empty());
    assert!(p.skips[0].reason.contains("schedule, workflow_dispatch"));
}

#[test]
fn unparsed_workflow_is_skipped_not_guessed() {
    let p = plan(&[("bad.yml", "on: push\njobs:\n  a: &anchor\n    runs-on: x\n")]);
    assert!(p.steps.is_empty());
    assert!(
        p.skips[0].reason.starts_with("unparsed: line 3"),
        "{}",
        p.skips[0].reason
    );
}

#[test]
fn matrix_lists_expand_and_filter_by_os() {
    let p = plan(&[(
        "m.yml",
        "on: pull_request\njobs:\n  t:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, macos-latest]\n        node: [18, 20]\n    steps:\n      - run: echo ${{ matrix.node }} on ${{ matrix.os }}\n",
    )]);
    let runs: Vec<&str> = p.steps.iter().map(|s| s.run.as_str()).collect();
    assert_eq!(
        runs,
        ["echo 18 on ubuntu-latest", "echo 20 on ubuntu-latest"]
    );
    assert_eq!(
        p.skips
            .iter()
            .filter(|s| s.reason.contains("macos"))
            .count(),
        2
    );
}

#[test]
fn computed_or_included_matrices_skip_the_job() {
    let p = plan(&[
        ("a.yml", "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    strategy:\n      matrix: ${{ fromJson(x) }}\n    steps:\n      - run: echo\n"),
        ("b.yml", "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    strategy:\n      matrix:\n        include:\n          - a: 1\n    steps:\n      - run: echo\n"),
    ]);
    assert!(p.steps.is_empty());
    assert_eq!(p.skips.len(), 2);
}

#[test]
fn defaults_working_directory_shell_and_timeout_flow_into_steps() {
    let p = plan(&[(
        "d.yml",
        "on: push\ndefaults:\n  run:\n    working-directory: mobile\n    shell: bash\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: npm test\n        timeout-minutes: 7\n      - run: ls\n        working-directory: other\n        continue-on-error: true\n",
    )]);
    assert_eq!(p.steps[0].cwd.as_deref(), Some("mobile"));
    assert_eq!(p.steps[0].shell.as_deref(), Some("bash"));
    assert_eq!(p.steps[0].timeout_secs, Some(420));
    assert_eq!(p.steps[1].cwd.as_deref(), Some("other"));
    assert!(p.steps[1].allow_failure);
}

#[test]
fn job_env_that_is_unknown_only_skips_steps_that_use_it() {
    let p = plan(&[(
        "e.yml",
        "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    env:\n      TOK: ${{ secrets.T }}\n    steps:\n      - run: echo plain\n      - run: curl -H \"$TOK\" x\n",
    )]);
    assert_eq!(p.steps.len(), 1);
    assert!(p.skips[0].reason.contains("needs secret T"));
}

#[test]
fn services_containers_environments_and_reusable_jobs_are_skipped() {
    let p = plan(&[(
        "s.yml",
        "on: push\njobs:\n  a:\n    runs-on: ubuntu-latest\n    services:\n      db:\n        image: postgres\n    steps:\n      - run: x\n  b:\n    runs-on: ubuntu-latest\n    container: node:20\n    steps:\n      - run: x\n  c:\n    runs-on: ubuntu-latest\n    environment: prod\n    steps:\n      - run: x\n  d:\n    uses: ./.github/workflows/r.yml\n",
    )]);
    assert!(p.steps.is_empty());
    let r: Vec<&str> = p.skips.iter().map(|s| s.reason.as_str()).collect();
    assert_eq!(
        r,
        [
            "needs service containers",
            "runs in a container",
            "deployment environment",
            "calls a reusable workflow"
        ]
    );
}

#[test]
fn runner_state_and_github_context_skip_the_step() {
    let p = plan(&[(
        "r.yml",
        "on: push\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo x >> $GITHUB_ENV\n      - run: echo ${{ github.sha }}\n      - run: echo ${{ runner.temp }}/f\n",
    )]);
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].run, "echo /tmp/rt/f");
    assert!(p.skips[0].reason.contains("GITHUB_ENV"));
    assert!(p.skips[1].reason.contains("github.sha"));
}

#[test]
fn paths_filter_skips_a_workflow_when_nothing_matches() {
    let src = "on:\n  pull_request:\n    paths:\n      - 'mobile/**'\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: x\n";
    let mut c = ctx();
    c.changed_files = Some(vec!["crates/a/src/lib.rs".into()]);
    let sources = vec![("m.yml".to_string(), Ok(src.to_string()))];
    assert!(plan_from_sources(&sources, &c, None).steps.is_empty());
    c.changed_files = Some(vec!["mobile/src/a.ts".into()]);
    assert_eq!(plan_from_sources(&sources, &c, None).steps.len(), 1);
}

#[test]
fn glob_semantics() {
    assert!(glob_match("mobile/**", "mobile/a/b.ts"));
    assert!(glob_match("**/*.rs", "crates/x/lib.rs"));
    assert!(glob_match("*.md", "README.md"));
    assert!(!glob_match("*.md", "docs/README.md"));
}

fn repo_plan() -> CiPlan {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    build_plan(&root, &ctx())
}

#[test]
fn this_repos_plan_carries_the_clippy_env_and_the_size_guard() {
    let p = repo_plan();
    let clippy = p
        .steps
        .iter()
        .find(|s| s.key == "ci.yml::clippy::bash scripts/ci/rust-checks.sh")
        .expect("clippy step");
    assert_eq!(
        clippy.env.get("RUSTFLAGS").map(String::as_str),
        Some("-D warnings")
    );
    assert!(p
        .steps
        .iter()
        .any(|s| s.run.contains("architecture_size.py") && s.name.starts_with("Enforce")));
    let test = p
        .steps
        .iter()
        .find(|s| s.run.starts_with("cargo test --locked --all"))
        .expect("cargo test");
    assert_eq!(
        test.env.get("FORGE_DB").map(String::as_str),
        Some("/tmp/rt/forge-ci-test.db")
    );
    assert!(p
        .steps
        .iter()
        .any(|s| s.run == "cargo fmt --all -- --check"));
}

#[test]
fn this_repos_plan_skips_runner_housekeeping_and_release_builds() {
    let p = repo_plan();
    assert!(!p
        .steps
        .iter()
        .any(|s| s.run.contains("cargo-cache-env.sh")
            || s.run == "bash scripts/ci/trim-runner-cache.sh"));
    assert!(p
        .skips
        .iter()
        .any(|s| s.job.as_deref() == Some("release-build")));
    assert!(
        !p.steps.iter().any(|s| shallow_git(&s.run).is_some()),
        "a step would make the owner's clone shallow"
    );
}

#[test]
fn scripts_that_write_runner_state_are_skipped_but_their_tests_are_not() {
    let dir = std::env::temp_dir().join(format!("forge-plan-scripts-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("setup.sh"),
        "echo A=1 >> \"${GITHUB_ENV:-/dev/stdout}\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("test-setup.sh"),
        "GITHUB_ENV=$(mktemp) bash setup.sh\n",
    )
    .unwrap();
    let wf = "on: push\njobs:\n  j:\n    runs-on: ubuntu-latest\n    steps:\n      - run: bash setup.sh\n      - run: bash test-setup.sh\n";
    let sources = vec![("w.yml".to_string(), Ok(wf.to_string()))];
    let p = plan_from_sources(&sources, &ctx(), Some(&dir));
    assert_eq!(p.steps.len(), 1);
    assert_eq!(p.steps[0].run, "bash test-setup.sh");
    assert!(p.skips[0].reason.contains("runner state"));
}

#[test]
fn a_fetch_that_makes_the_clone_shallow_is_skipped_and_a_plain_fetch_is_not() {
    let wf = r#"
on: push
jobs:
  j:
    runs-on: ubuntu-latest
    steps:
      - name: Guard
        run: |
          git fetch --no-tags --depth=1 origin main
          ./scripts/guard.sh
      - name: Tag
        run: git fetch --depth 1 origin "refs/tags/v1:refs/tags/v1"
      - name: Since
        run: git -C . pull --shallow-since=2024-01-01
      - name: Plain fetch
        run: git fetch --no-tags origin main
      - name: Tree
        run: cargo tree --depth 1
"#;
    let p = plan(&[("ci.yml", wf)]);
    let runnable: Vec<&str> = p.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(runnable, ["Plain fetch", "Tree"], "{:#?}", p.skips);
    for name in ["Guard", "Tag", "Since"] {
        assert!(
            p.skips
                .iter()
                .any(|k| k.step.as_deref() == Some(name) && k.reason.contains("shallow")),
            "{name}: {:#?}",
            p.skips
        );
    }
}

const MAIN_ONLY_DEPLOY: &str = "on:\n  push:\n    branches: [main]\n    tags: ['v*']\njobs:\n  deploy:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo built\n";

#[test]
fn push_branch_and_tag_filters_follow_the_current_branch() {
    let main_only = plan(&[("deploy.yml", MAIN_ONLY_DEPLOY)]);
    assert!(main_only.steps.is_empty(), "{:?}", main_only.steps);
    assert!(main_only.skips[0].job.is_none());

    let tags_only = plan(&[(
        "release.yml",
        "on:\n  push:\n    tags: ['v*']\njobs:\n  r:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo release\n",
    )]);
    assert!(tags_only.steps.is_empty(), "{:?}", tags_only.steps);

    let mut on_main = ctx();
    on_main.branch = Some("main".to_string());
    let src = vec![("deploy.yml".to_string(), Ok(MAIN_ONLY_DEPLOY.to_string()))];
    assert_eq!(plan_from_sources(&src, &on_main, None).steps.len(), 1);

    let wildcard = plan(&[(
        "ci.yml",
        "on:\n  push:\n    branches: ['feature/**', '!feature/skip']\njobs:\n  t:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo test\n",
    )]);
    assert_eq!(wildcard.steps.len(), 1);
}

#[test]
fn job_gated_on_the_ref_is_skipped_and_deploy_tools_are_never_run() {
    let p = plan(&[(
        "cd.yml",
        "on: push\njobs:\n  deploy:\n    if: github.ref == 'refs/heads/main'\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo gated\n  tools:\n    runs-on: ubuntu-latest\n    steps:\n      - run: aws ecs update-service --cluster prod --force-new-deployment\n      - run: terraform -chdir=infra apply -auto-approve\n      - run: kubectl apply -f k8s/\n      - run: ./deploy.sh\n",
    )]);
    assert!(p.steps.is_empty(), "{:?}", p.steps);
    assert!(p
        .skips
        .iter()
        .any(|s| s.reason.starts_with("conditional job")));
}
