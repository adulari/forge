use super::*;
use forge_skills::{Scope, ScopedDir, Sources};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("forge-skills-{name}-{}", forge_types::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_skill(root: &Path, name: &str) {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: does {name}\n---\nbody"),
    )
    .unwrap();
}

fn sources(user: &Path, project: &Path) -> Sources {
    Sources {
        commands: vec![],
        skills: vec![
            ScopedDir {
                scope: Scope::User,
                path: user.to_path_buf(),
            },
            ScopedDir {
                scope: Scope::Project,
                path: project.to_path_buf(),
            },
        ],
    }
}

#[test]
fn sync_links_user_and_project_skills_for_bridged_agents() {
    let user = temp("user");
    let project = temp("project");
    let base = temp("base");
    write_skill(&user, "alpha");
    write_skill(&project, "beta");
    let report = sync_skills(&sources(&user, &project), &base, false);
    assert_eq!(report.linked.len(), 4);
    assert!(report.conflicts.is_empty());
    for dir in [".agents/skills", ".claude/skills"] {
        for name in ["alpha", "beta"] {
            let link = base.join(dir).join(name);
            assert!(link.join("SKILL.md").is_file(), "{}", link.display());
        }
    }
    let again = sync_skills(&sources(&user, &project), &base, false);
    assert_eq!(again.linked.len(), 4);
}

#[test]
fn sync_user_scope_skips_project_skills_and_keeps_real_directories() {
    let user = temp("user2");
    let project = temp("project2");
    let base = temp("base2");
    write_skill(&user, "alpha");
    write_skill(&project, "beta");
    write_skill(&base.join(".agents/skills"), "alpha");
    let report = sync_skills(&sources(&user, &project), &base, true);
    assert_eq!(report.conflicts.len(), 1);
    assert!(!base.join(".agents/skills/alpha").is_symlink());
    assert!(base.join(".claude/skills/alpha").is_symlink());
    assert!(!base.join(".claude/skills/beta").exists());
}

#[tokio::test]
async fn installs_from_a_local_git_marketplace_and_removes_again() {
    let repo = temp("repo");
    write_skill(&repo, "pack-one");
    std::fs::write(
        repo.join("pack-one/SKILL.md"),
        "---\nname: pack-one\ndescription: first pack\n---\nbody",
    )
    .unwrap();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        let ok = std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success();
        assert!(ok);
    }
    let url = format!("file://{}", repo.display());
    let found = list_available(&url, None).await.unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].name, "pack-one");
    assert_eq!(found[0].description, "first pack");

    let skills = temp("dest");
    let lock = skills.join("installed.toml");
    let mut registry = Marketplaces::default();
    registry.marketplaces.insert(
        "fixture".into(),
        MarketplaceEntry {
            source: url,
            git_ref: None,
        },
    );
    let report = install_pack_to(
        "pack-one",
        Some("fixture"),
        &registry,
        &skills.join("skills"),
        &lock,
    )
    .await
    .unwrap();
    assert_eq!(report.installed, vec!["pack-one".to_string()]);
    assert!(skills.join("skills/pack-one/SKILL.md").is_file());
    assert!(remove_plugin_at(&skills.join("skills"), &lock, "pack-one").unwrap());
    assert!(!skills.join("skills/pack-one").exists());
}
