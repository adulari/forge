use super::*;
use crate::cli::commands::marketplace::{record_installed_at, InstalledEntry};
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

fn git_fixture(files: &[(&str, &str)]) -> String {
    let repo = temp("fixture");
    for (path, body) in files {
        let full = repo.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
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
        assert!(std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    format!("file://{}", repo.display())
}

fn registry_for(url: String) -> Marketplaces {
    let mut registry = Marketplaces::default();
    registry.marketplaces.insert(
        "fixture".into(),
        MarketplaceEntry {
            source: url,
            git_ref: None,
        },
    );
    registry
}

fn write_named(root: &Path, dir: &str, name: &str) {
    let d = root.join(dir);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: x\n---\nbody"),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn sync_never_links_outside_the_skills_dir_for_hostile_names() {
    let user = temp("hostile-user");
    let project = temp("hostile-project");
    let base = temp("hostile-base");
    let victim = base.join("victim");
    std::fs::create_dir_all(&victim).unwrap();
    std::os::unix::fs::symlink("/etc/hostname", victim.join("dotfile")).unwrap();
    write_named(&user, "up", "../../escaped");
    write_named(&user, "abs", &format!("{}/abs-escape", base.display()));
    write_named(&user, "stow", "../../victim/dotfile");
    let report = sync_skills(&sources(&user, &project), &base, true);
    assert!(report.linked.is_empty(), "{report:?}");
    assert!(std::fs::symlink_metadata(base.join("escaped")).is_err());
    assert!(std::fs::symlink_metadata(base.join("abs-escape")).is_err());
    assert_eq!(
        std::fs::read_link(victim.join("dotfile")).unwrap(),
        Path::new("/etc/hostname")
    );
}

#[cfg(unix)]
#[test]
fn sync_keeps_a_symlink_the_user_manages() {
    let user = temp("keep-user");
    let project = temp("keep-project");
    let base = temp("keep-base");
    let dotfiles = temp("keep-dotfiles");
    write_skill(&user, "review");
    write_skill(&dotfiles, "review");
    std::fs::create_dir_all(base.join(".claude/skills")).unwrap();
    let theirs = base.join(".claude/skills/review");
    std::os::unix::fs::symlink(dotfiles.join("review"), &theirs).unwrap();
    let report = sync_skills(&sources(&user, &project), &base, true);
    assert_eq!(report.conflicts, vec![theirs.display().to_string()]);
    assert_eq!(
        std::fs::read_link(&theirs).unwrap(),
        dotfiles.join("review")
    );
    assert!(base.join(".agents/skills/review").is_symlink());
}

#[cfg(unix)]
#[test]
fn sync_repoints_a_stale_forge_link() {
    let user = temp("stale-user");
    let project = temp("stale-project");
    let base = temp("stale-base");
    write_skill(&user, "alpha");
    std::fs::create_dir_all(base.join(".agents/skills")).unwrap();
    std::os::unix::fs::symlink(user.join("gone"), base.join(".agents/skills/alpha")).unwrap();
    let report = sync_skills(&sources(&user, &project), &base, true);
    assert!(report.conflicts.is_empty(), "{report:?}");
    assert_eq!(
        std::fs::read_link(base.join(".agents/skills/alpha")).unwrap(),
        user.join("alpha")
    );
}

#[tokio::test]
async fn reinstall_succeeds_and_reports_kept_skills_honestly() {
    let url = git_fixture(&[(
        "pack-one/SKILL.md",
        "---\nname: pack-one\ndescription: first\n---\nbody",
    )]);
    let registry = registry_for(url);
    let dest = temp("reinstall");
    let skills = dest.join("skills");
    let lock = dest.join("installed.toml");
    install_pack_to("pack-one", Some("fixture"), &registry, &skills, &lock)
        .await
        .unwrap();
    let again = install_pack_to("pack-one", Some("fixture"), &registry, &skills, &lock)
        .await
        .unwrap();
    assert_eq!(again.installed, vec!["pack-one".to_string()]);

    let hand = temp("reinstall-hand");
    write_skill(&hand.join("skills"), "pack-one");
    let err = install_pack_to(
        "pack-one",
        Some("fixture"),
        &registry,
        &hand.join("skills"),
        &hand.join("installed.toml"),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("kept your existing"), "{err}");
}

#[tokio::test]
async fn flat_markdown_package_installs() {
    let url = git_fixture(&[(
        "skills/pirate.md",
        "---\nname: pirate\ndescription: arr\n---\nbody",
    )]);
    let registry = registry_for(url.clone());
    assert_eq!(list_available(&url, None).await.unwrap()[0].name, "pirate");
    let dest = temp("flat");
    let report = install_pack_to(
        "pirate",
        Some("fixture"),
        &registry,
        &dest.join("skills"),
        &dest.join("installed.toml"),
    )
    .await
    .unwrap();
    assert_eq!(report.installed, vec!["pirate".to_string()]);
    assert!(dest.join("skills/pirate/SKILL.md").is_file());
}

#[cfg(unix)]
#[tokio::test]
async fn install_skips_symlinked_entries() {
    let secret = temp("secret");
    std::fs::write(secret.join("private.md"), "---\nname: x\n---\nSECRET").unwrap();
    write_skill(&secret, "linkdir");
    let repo = temp("symlink-repo");
    write_skill(&repo, "real");
    std::os::unix::fs::symlink(secret.join("private.md"), repo.join("stolen.md")).unwrap();
    std::os::unix::fs::symlink(secret.join("linkdir"), repo.join("linkdir")).unwrap();
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
        assert!(std::process::Command::new("git")
            .args(&args)
            .current_dir(&repo)
            .status()
            .unwrap()
            .success());
    }
    let dest = temp("symlink-dest");
    let report = install_pack_to(
        &format!("file://{}", repo.display()),
        None,
        &Marketplaces::default(),
        &dest.join("skills"),
        &dest.join("installed.toml"),
    )
    .await
    .unwrap();
    assert_eq!(report.installed, vec!["real".to_string()]);
    assert!(!dest.join("skills/stolen").exists());
    assert!(!dest.join("skills/linkdir").exists());
}

#[test]
fn removing_a_pack_keeps_an_unowned_skill_with_the_packs_name() {
    let dir = temp("remove-owned");
    let skills = dir.join("skills");
    let lock = dir.join("installed.toml");
    write_skill(&skills, "lint");
    write_skill(&skills, "code-review");
    record_installed_at(
        &lock,
        "code-review",
        InstalledEntry {
            source: "x/y".into(),
            marketplace: None,
            subdir: None,
            git_ref: None,
            files: vec!["lint".into()],
        },
    )
    .unwrap();
    assert!(remove_plugin_at(&skills, &lock, "code-review").unwrap());
    assert!(!skills.join("lint").exists());
    assert!(skills.join("code-review/SKILL.md").is_file());
}
