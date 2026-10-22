//! Claude Code invocation frontmatter consumed by the catalog (G9 of the CC parity audit).

use super::Tmp;
use crate::*;

#[test]
fn disable_model_invocation_hides_a_skill_from_the_model_only() {
    let t = Tmp::new();
    t.skill(
        "user",
        "deploy",
        "---\ndescription: ship it\ndisable-model-invocation: true\n---\nDeploy now.",
    );
    let cat = t.load();
    assert!(cat.skill_listing(true).iter().all(|(n, _)| n != "deploy"));
    assert_eq!(cat.skill_guidance("deploy", true), None);
    assert!(cat.entries().iter().any(|e| e.name == "deploy"));
    assert!(matches!(
        cat.resolve("/deploy prod"),
        Resolved::Skill { .. }
    ));
}

#[test]
fn user_invocable_false_hides_from_the_palette_and_refuses_typing() {
    let t = Tmp::new();
    t.skill(
        "user",
        "house-style",
        "---\ndescription: background conventions\nuser-invocable: false\n---\nUse tabs.",
    );
    t.cmd(
        "user",
        "internal",
        "---\nuser-invocable: no\n---\nInternal prompt.",
    );
    let cat = t.load();
    assert!(cat
        .entries()
        .iter()
        .all(|e| e.name != "house-style" && e.name != "internal"));
    assert_eq!(
        cat.resolve("/house-style"),
        Resolved::NotUserInvocable("house-style".into())
    );
    assert_eq!(
        cat.resolve("/skill house-style"),
        Resolved::NotUserInvocable("house-style".into())
    );
    assert_eq!(
        cat.resolve("/internal x"),
        Resolved::NotUserInvocable("internal".into())
    );
    assert!(cat
        .skill_listing(true)
        .iter()
        .any(|(n, _)| n == "house-style"));
    assert!(cat
        .skill_guidance("house-style", true)
        .unwrap()
        .contains("Use tabs."));
}

#[test]
fn argument_hint_reaches_the_palette_entry() {
    let t = Tmp::new();
    t.cmd(
        "user",
        "fix-issue",
        "---\nargument-hint: [issue-number]\n---\nFix issue $ARGUMENTS",
    );
    t.cmd(
        "user",
        "derived",
        "---\nargs: [path, note?]\n---\nLook at $path",
    );
    t.skill(
        "user",
        "explain",
        "---\ndescription: explain\nargument-hint: \"<file> [depth]\"\n---\nExplain.",
    );
    let cat = t.load();
    let hint = |name: &str| {
        cat.entries()
            .into_iter()
            .find(|e| e.name == name)
            .unwrap()
            .hint
    };
    assert_eq!(hint("fix-issue"), "[issue-number]");
    assert_eq!(hint("derived"), "<path> [note]");
    assert_eq!(hint("explain"), "<file> [depth]");
}

#[test]
fn claude_code_zero_based_command_with_quoted_arguments() {
    let t = Tmp::new();
    t.cmd(
        "user",
        "pair",
        "---\nallowed-tools: Bash(git log:*)\n---\nfirst=$0 second=$1 again=$ARGUMENTS[1] all=$ARGUMENTS",
    );
    let cat = t.load();
    let cmd = cat.command("pair").unwrap();
    assert_eq!(cmd.invocation.allowed_tools, ["Bash(git log:*)"]);
    match cat.resolve(r#"/pair "hello world" two"#) {
        Resolved::Command { prompt, .. } => assert_eq!(
            prompt,
            r#"first=hello world second=two again=two all="hello world" two"#
        ),
        other => panic!("got {other:?}"),
    }
}

#[test]
fn claude_code_arguments_are_optional_named_args() {
    let t = Tmp::new();
    t.cmd(
        "user",
        "review-pr",
        "---\narguments: [pr, focus]\n---\nReview PR #$pr focusing on [$focus].",
    );
    let cat = t.load();
    match cat.resolve("/review-pr 12") {
        Resolved::Command { prompt, .. } => {
            assert_eq!(prompt, "Review PR #12 focusing on [].")
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn session_variables_expand_in_commands_and_skills() {
    let t = Tmp::new();
    t.cmd(
        "user",
        "log",
        "Write to /tmp/${CLAUDE_SESSION_ID}.log in ${CLAUDE_PROJECT_DIR}",
    );
    t.skill(
        "user",
        "chart",
        "---\ndescription: chart\narguments: csv\n---\nRun ${CLAUDE_SKILL_DIR}/render.sh $csv for ${FORGE_SESSION_ID}",
    );
    let cat = t.load();
    let vars = TemplateVars {
        session_id: Some("abc".into()),
        project_dir: Some("/repo".into()),
        ..TemplateVars::default()
    };
    match cat.resolve_with("/log", &vars) {
        Resolved::Command { prompt, .. } => {
            assert_eq!(prompt, "Write to /tmp/abc.log in /repo")
        }
        other => panic!("got {other:?}"),
    }
    let Resolved::Skill { meta, prompt } = cat.resolve_with("/chart data.csv", &vars) else {
        panic!("chart is a skill");
    };
    let rendered = Skill::load(&meta).render(Some(&prompt), &vars);
    let dir = meta.dir.display().to_string();
    assert_eq!(rendered, format!("Run {dir}/render.sh data.csv for abc"));
    // The model-facing load has no argument text: placeholders stay, variables expand.
    let model = cat.skill_guidance_with("chart", true, &vars).unwrap();
    assert_eq!(model, format!("Run {dir}/render.sh $csv for abc"));
}

#[test]
fn fork_paths_and_effort_are_recorded_on_the_skill() {
    let t = Tmp::new();
    t.skill(
        "user",
        "pr-summary",
        "---\ndescription: summarize a PR\ncontext: fork\nagent: Explore\nmodel: haiku\npaths:\n  - src/**\neffort: low\ndisallowed-tools: AskUserQuestion\n---\nSummarize.",
    );
    let cat = t.load();
    let meta = cat.skill("pr-summary").unwrap();
    assert_eq!(
        meta.invocation.fork,
        Some(ForkSpec {
            agent: Some("Explore".into())
        })
    );
    assert_eq!(meta.model.as_deref(), Some("haiku"));
    assert_eq!(meta.invocation.effort.as_deref(), Some("low"));
    assert_eq!(meta.invocation.disallowed_tools, ["AskUserQuestion"]);
    assert_eq!(cat.skill_paths("pr-summary"), ["src/**"]);
    assert!(cat.skill_paths("missing").is_empty());
}
