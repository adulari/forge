use std::os::unix::fs::PermissionsExt;

use super::*;

const YAML: &str = r#"name: Release
on:
  push:
    branches: [main]
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to cut'
        required: true
        type: string
      channel:
        description: Where to publish
        default: stable
        type: choice
        options:
          - stable
          - "beta"
      dry_run:
        type: boolean
        default: false
jobs:
  build:
    runs-on: ubuntu-latest
"#;

fn fake_gh(dir: &Path, script: &str) -> Gh {
    let path = dir.join("gh");
    std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    Gh::with_program(path)
}

#[test]
fn dispatch_inputs_are_read_from_workflow_yaml() {
    let inputs = parse_dispatch_inputs(YAML);
    assert_eq!(inputs.len(), 3);
    assert_eq!(inputs[0].name, "version");
    assert!(inputs[0].required);
    assert_eq!(inputs[0].description, "Version to cut");
    assert_eq!(inputs[1].options, vec!["stable", "beta"]);
    assert_eq!(inputs[1].default.as_deref(), Some("stable"));
    assert_eq!(inputs[2].kind, "boolean");
    assert!(parse_dispatch_inputs("on:\n  push:\n").is_empty());
    assert!(parse_dispatch_inputs("on:\n  workflow_dispatch:\njobs:\n  a:\n").is_empty());
}

#[test]
fn identifiers_are_validated_before_reaching_gh() {
    assert!(valid_workflow("ci.yml") && valid_workflow("Release build"));
    assert!(!valid_workflow("--repo=evil/x") && !valid_workflow(""));
    assert!(valid_ref("feature/x-1") && !valid_ref("-x") && !valid_ref("a..b"));
    assert!(valid_input_key("dry_run") && !valid_input_key("a=b"));
}

#[tokio::test]
async fn dispatch_passes_ref_and_inputs_as_separate_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let gh = fake_gh(
        dir.path(),
        &format!(
            "for a in \"$@\"; do echo \"[$a]\"; done >> {}",
            log.display()
        ),
    );
    let mut inputs = Map::new();
    inputs.insert("version".into(), json!("1.2.3 beta"));
    inputs.insert("dry_run".into(), json!(true));
    let request = DispatchRequest {
        session: "s".into(),
        workflow: "release.yml".into(),
        git_ref: Some("main".into()),
        inputs,
    };
    dispatch_in(&gh, dir.path(), &request).await.unwrap();
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(logged.contains("[workflow]\n[run]\n[release.yml]\n[--ref]\n[main]\n"));
    assert!(logged.contains("[-f]\n[version=1.2.3 beta]"));
    assert!(logged.contains("[dry_run=true]"));

    let bad = DispatchRequest {
        session: "s".into(),
        workflow: "release.yml".into(),
        git_ref: Some("--upload-pack=x".into()),
        inputs: Map::new(),
    };
    assert!(dispatch_in(&gh, dir.path(), &bad).await.is_err());
}

#[tokio::test]
async fn run_verbs_map_to_gh_run_subcommands() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("args.log");
    let gh = fake_gh(dir.path(), &format!("echo \"$@\" >> {}", log.display()));
    run_verb(&gh, dir.path(), "rerun", 5, true).await.unwrap();
    run_verb(&gh, dir.path(), "cancel", 6, true).await.unwrap();
    run_verb(&gh, dir.path(), "delete", 7, false).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        "run rerun 5 --failed\nrun cancel 6\nrun delete 7\n"
    );
}

#[tokio::test]
async fn run_listing_rejects_option_like_workflow_names() {
    let dir = tempfile::tempdir().unwrap();
    let gh = fake_gh(dir.path(), "echo '[]'");
    assert!(list_runs(&gh, dir.path(), Some("--json"), None)
        .await
        .is_err());
    assert_eq!(
        list_runs(&gh, dir.path(), Some("ci.yml"), Some(500))
            .await
            .unwrap(),
        json!([])
    );
}
