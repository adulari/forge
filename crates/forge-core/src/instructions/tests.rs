use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A temp tree read from its own top, never above it.
fn tree() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    (tmp, top)
}

fn locations(top: &Path) -> Locations {
    Locations {
        project: true,
        top: Some(top.to_path_buf()),
        ..Locations::default()
    }
}

fn bodies(loaded: &Loaded) -> Vec<&str> {
    loaded.files.iter().map(|f| f.body.trim()).collect()
}

#[test]
fn a_project_file_no_longer_hides_the_users_global_file() {
    let (_tmp, top) = tree();
    let global = top.join("home/.claude/CLAUDE.md");
    write(&global, "GLOBAL: be brief\n");
    let root = top.join("repo");
    write(&root.join("AGENTS.md"), "PROJECT: run cargo test\n");
    let mut locations = locations(&root);
    locations.global_files.push((global, true));
    let loaded = load_with(&root, &locations);
    assert_eq!(
        bodies(&loaded),
        ["GLOBAL: be brief", "PROJECT: run cargo test"]
    );
    assert_eq!(loaded.files[0].origin, Origin::User);
    assert!(loaded.files[0].claude_loads && !loaded.files[1].claude_loads);
}

#[test]
fn parents_apply_root_down_and_every_file_of_a_directory_counts() {
    let (_tmp, top) = tree();
    let root = top.join("pkg/app");
    write(&top.join("CLAUDE.md"), "TOP");
    write(&top.join("pkg/AGENTS.md"), "PKG");
    write(&root.join(".forge/AGENTS.md"), "APP-FORGE");
    write(&root.join("AGENTS.md"), "APP-AGENTS");
    write(&root.join(".claude/CLAUDE.md"), "APP-DOT-CLAUDE");
    write(&root.join("CLAUDE.local.md"), "APP-LOCAL");
    let loaded = load_with(&root, &locations(&top));
    assert_eq!(
        bodies(&loaded),
        [
            "TOP",
            "PKG",
            "APP-FORGE",
            "APP-AGENTS",
            "APP-DOT-CLAUDE",
            "APP-LOCAL"
        ]
    );
    assert_eq!(loaded.files[5].origin, Origin::Local);
    // Nothing above `top` is read.
    assert!(loaded.watched.iter().all(|p| p.starts_with(&top)));
}

#[cfg(unix)]
#[test]
fn a_parent_anyone_can_write_to_is_skipped() {
    use std::os::unix::fs::PermissionsExt;
    let (_tmp, top) = tree();
    let shared = top.join("shared");
    let root = shared.join("work");
    write(&shared.join("CLAUDE.md"), "PLANTED");
    write(&root.join("AGENTS.md"), "MINE");
    std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o777)).unwrap();
    assert_eq!(bodies(&load_with(&root, &locations(&top))), ["MINE"]);
}

#[test]
fn imports_follow_the_importer_up_to_five_levels() {
    let (_tmp, root) = tree();
    write(&root.join("CLAUDE.md"), "ROOT\nSee @docs/l1.md for more.");
    for level in 1..=6 {
        write(
            &root.join(format!("docs/l{level}.md")),
            &format!("LEVEL{level}\n@l{}.md\n", level + 1),
        );
    }
    let loaded = load_with(&root, &locations(&root));
    assert_eq!(
        bodies(&loaded),
        [
            "ROOT\nSee @docs/l1.md for more.",
            "LEVEL1",
            "LEVEL2",
            "LEVEL3",
            "LEVEL4",
            "LEVEL5"
        ]
    );
    assert_eq!(
        loaded.files[1].via.as_deref(),
        Some(root.join("CLAUDE.md").as_path())
    );
    assert!(
        loaded.files.iter().all(|f| f.claude_loads),
        "imports of a CLAUDE.md are read by a bridged claude too"
    );
}

#[test]
fn an_at_sign_in_code_or_an_email_is_not_an_import() {
    let (_tmp, root) = tree();
    write(&root.join("x.md"), "IMPORTED");
    write(
        &root.join("AGENTS.md"),
        "use `@x.md` literally\n```\n@x.md\n```\nmail me@x.md\n",
    );
    let loaded = load_with(&root, &locations(&root));
    assert_eq!(loaded.files.len(), 1);
}

#[test]
fn a_project_file_imports_only_inside_its_repository_and_never_a_secret() {
    let (_tmp, top) = tree();
    let repo = top.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    write(&top.join("outside.md"), "OUTSIDE");
    write(&repo.join(".env"), "API_KEY=secret");
    write(&repo.join("inside.md"), "INSIDE");
    write(
        &repo.join("CLAUDE.md"),
        "RULES\n@../outside.md\n@.env\n@inside.md\n",
    );
    let loaded = load_with(&repo, &locations(&repo));
    assert_eq!(bodies(&loaded), ["RULES\n@../outside.md\n@.env", "INSIDE"]);
}

#[test]
fn html_comments_are_dropped() {
    let (_tmp, root) = tree();
    write(
        &root.join("AGENTS.md"),
        "keep <!-- inline --> this\n<!--\nwhole\nblock\n-->\nend\n```\n<!-- in code -->\n```",
    );
    let loaded = load_with(&root, &locations(&root));
    assert_eq!(
        bodies(&loaded),
        ["keep  this\nend\n```\n<!-- in code -->\n```"]
    );
}

#[test]
fn a_file_reached_twice_is_carried_once() {
    let (_tmp, root) = tree();
    write(&root.join("AGENTS.md"), "SHARED RULES");
    write(&root.join("CLAUDE.md"), "@AGENTS.md\n");
    write(&root.join(".claude/CLAUDE.md"), "SHARED RULES\n");
    let loaded = load_with(&root, &locations(&root));
    assert_eq!(bodies(&loaded), ["SHARED RULES"]);
    assert!(
        loaded.files[0].claude_loads,
        "claude reads it through the CLAUDE.md import"
    );
}

#[test]
fn rules_without_paths_load_and_scoped_rules_wait_for_a_match() {
    let (_tmp, root) = tree();
    write(&root.join(".claude/rules/style.md"), "STYLE: tabs");
    write(
        &root.join(".claude/rules/api/handlers.md"),
        "---\npaths:\n  - \"src/api/**/*.ts\"\n---\nAPI: validate input",
    );
    write(&root.join(".forge/rules/forge-only.md"), "FORGE RULE");
    let loaded = load_with(&root, &locations(&root));
    assert_eq!(bodies(&loaded), ["STYLE: tabs", "FORGE RULE"]);
    assert!(loaded.files[0].claude_loads && !loaded.files[1].claude_loads);
    let [rule] = loaded.scoped_rules.as_slice() else {
        panic!("one scoped rule: {:?}", loaded.scoped_rules);
    };
    assert!(rule.matches(&root.join("src/api/v1/users.ts"), &root));
    assert!(!rule.matches(&root.join("src/web/users.ts"), &root));
    assert!(rule.to_file().body.contains("API: validate input"));
}

#[test]
fn the_claude_bridge_is_sent_only_what_claude_does_not_read() {
    let (_tmp, root) = tree();
    write(&root.join("AGENTS.md"), "AGENTS ONLY");
    write(&root.join("CLAUDE.md"), "CLAUDE ONLY");
    let loaded = load_with(&root, &locations(&root));
    let direct = render(&loaded.files, &root).unwrap();
    assert!(direct.contains("AGENTS ONLY") && direct.contains("CLAUDE ONLY"));
    assert!(direct.contains("Contents of CLAUDE.md (project instructions)"));
    let bridged = render(loaded.files.iter().filter(|f| !f.claude_loads), &root).unwrap();
    assert_eq!(bridged, "AGENTS ONLY", "one file reads exactly as before");
}

#[test]
fn without_the_project_source_only_global_files_load() {
    let (_tmp, root) = tree();
    write(&root.join("AGENTS.md"), "PROJECT");
    let loaded = load_with(&root, &Locations::default());
    assert!(loaded.files.is_empty() && loaded.watched.is_empty());
}

#[test]
fn the_fingerprint_moves_when_a_watched_file_appears_or_changes() {
    let (_tmp, root) = tree();
    let loaded = load_with(&root, &locations(&root));
    let before = fingerprint(&loaded.watched);
    write(&root.join("CLAUDE.local.md"), "NEW");
    assert_ne!(before, fingerprint(&loaded.watched));
    let loaded = load_with(&root, &locations(&root));
    let now = fingerprint(&loaded.watched);
    assert_eq!(now, fingerprint(&loaded.watched));
    write(&root.join(".claude/rules/new.md"), "RULE");
    assert_ne!(
        now,
        fingerprint(&loaded.watched),
        "a new rule directory is seen"
    );
}

#[test]
fn dirs_below_the_root_run_outermost_first() {
    let root = Path::new("/w");
    assert_eq!(
        dirs_below(root, Path::new("/w/a/b/c.rs")),
        [PathBuf::from("/w/a"), PathBuf::from("/w/a/b")]
    );
    assert!(dirs_below(root, Path::new("/w/c.rs")).is_empty());
    assert!(dirs_below(root, Path::new("/elsewhere/c.rs")).is_empty());
}

fn session(root: &Path) -> crate::Session {
    crate::Session::start(
        Arc::new(forge_store::Store::open_in_memory().unwrap()),
        Arc::new(forge_provider::MockProvider),
        Arc::new(forge_mesh::HeuristicRouter::new(
            forge_config::Config::default(),
        )),
        forge_tools::ToolRegistry::with_core_tools_in(root),
        Box::new(forge_tui::HeadlessPresenter::new(false)),
        forge_config::Config::default(),
        root.to_str().unwrap(),
    )
    .unwrap()
}

#[test]
fn a_subdirectory_file_arrives_with_the_first_read_under_it_and_again_after_a_compaction() {
    let (_tmp, root) = tree();
    write(&root.join("AGENTS.md"), "ROOT RULES");
    write(&root.join("pkg/CLAUDE.md"), "PKG RULES\n@../AGENTS.md\n");
    write(
        &root.join(".claude/rules/rs.md"),
        "---\npaths: \"**/*.rs\"\n---\nRUST RULE",
    );
    let mut session = session(&root);
    let read = |session: &mut crate::Session, path: &str| {
        session.with_scoped_instructions(&[path.to_string()], "CONTENT".into())
    };
    assert_eq!(read(&mut session, "README.md"), "CONTENT");
    let first = read(&mut session, "pkg/lib.rs");
    assert!(first.starts_with("CONTENT\n\n[instructions]"), "{first}");
    assert!(first.contains("PKG RULES") && first.contains("RUST RULE"));
    assert!(
        !first.contains("ROOT RULES"),
        "the pinned block already carries the root file"
    );
    assert_eq!(read(&mut session, "pkg/other.rs"), "CONTENT");
    write(&root.join("node_modules/dep/CLAUDE.md"), "THIRD PARTY");
    assert_eq!(read(&mut session, "node_modules/dep/index.js"), "CONTENT");
    session.instruction_scope = Default::default();
    assert!(read(&mut session, "pkg/lib.rs").contains("PKG RULES"));
}

#[test]
fn a_session_pins_both_files_for_a_direct_model_and_only_agents_md_for_the_claude_bridge() {
    let (_tmp, root) = tree();
    write(&root.join("AGENTS.md"), "AGENTS ONLY");
    write(&root.join("CLAUDE.md"), "CLAUDE ONLY");
    let session = session(&root);
    let direct = session.pinned_context_message(Some("m::x")).unwrap();
    assert!(direct.content.contains("AGENTS ONLY") && direct.content.contains("CLAUDE ONLY"));
    let bridged = session
        .pinned_context_message(Some("claude-cli::sonnet"))
        .unwrap();
    assert!(bridged.content.contains("AGENTS ONLY") && !bridged.content.contains("CLAUDE ONLY"));
}
