//! Git-backed skill package installation and update.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

mod registry;
pub(crate) use registry::{
    add_marketplace_at, installed_path, load_installed_at, load_marketplaces_at, marketplaces_path,
    record_installed_at, remove_installed_entry, remove_marketplace_at, skills_dir, InstalledEntry,
    Marketplaces,
};

/// A private-repo / authenticated token, if the user exported one.
fn github_token() -> Option<String> {
    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .ok()
        .filter(|t| !t.is_empty())
}

// ---------------------------------------------------------------------------
// Marketplace commands
// ---------------------------------------------------------------------------

pub(crate) fn marketplace_add(name: &str, source: &str, git_ref: Option<String>) -> Result<()> {
    let path = marketplaces_path()?;
    let replaced = add_marketplace_at(&path, name, source, git_ref.clone())?;
    let pin = git_ref.map(|r| format!(" @{r}")).unwrap_or_default();
    if replaced {
        println!("✓ updated marketplace '{name}' → {source}{pin}");
    } else {
        println!("✓ added marketplace '{name}' → {source}{pin}");
    }
    println!("  install from it with: forge plugin install <pkg>@{name}");
    Ok(())
}

pub(crate) fn marketplace_list() -> Result<()> {
    let path = marketplaces_path()?;
    let m = load_marketplaces_at(&path)?;
    if m.marketplaces.is_empty() {
        println!("no marketplaces configured — add one with `forge plugin marketplace add <name> <source>`");
        return Ok(());
    }
    println!("configured marketplaces ({}):", m.marketplaces.len());
    for (name, entry) in &m.marketplaces {
        let pin = entry
            .git_ref
            .as_deref()
            .map(|r| format!(" @{r}"))
            .unwrap_or_default();
        println!("  {name}  →  {}{pin}", entry.source);
    }
    Ok(())
}

pub(crate) fn marketplace_remove(name: &str) -> Result<()> {
    let path = marketplaces_path()?;
    if remove_marketplace_at(&path, name)? {
        println!("✓ removed marketplace '{name}'");
    } else {
        println!("no marketplace '{name}' configured");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Resolution: turn a `pkg`/`pkg@marketplace`/`owner/repo[@ref]`/URL into a clone target
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// owner/repo or a full git URL to clone.
    pub(crate) clone_target: String,
    /// Subdirectory within the repo holding the package (marketplace installs), if any.
    pub(crate) subdir: Option<String>,
    /// Pinned branch/tag/ref, if any.
    pub(crate) git_ref: Option<String>,
    /// The lockfile key / display name for the pack.
    pub(crate) name: String,
    /// The marketplace it resolved through, if any.
    pub(crate) marketplace: Option<String>,
}

/// Derive a short pack name from an `owner/repo` or git URL (the repo's last path segment).
fn derive_pkg_name(target: &str) -> String {
    target
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(target)
        .trim_end_matches(".git")
        .to_string()
}

/// Pure resolution against a registry. `pkg` accepts:
/// - `owner/repo` / `owner/repo@ref` / a full git URL → install the whole repo.
/// - `pkg@marketplace` (where `marketplace` is registered) → the `pkg` subdir of that marketplace.
/// - a bare `pkg` together with `marketplace_flag` → the same, marketplace from the flag.
pub(crate) fn resolve(
    pkg: &str,
    marketplace_flag: Option<&str>,
    registry: &Marketplaces,
) -> Result<Resolved> {
    let pkg = pkg.trim();
    // A full URL never gets @-split (it has no marketplace/ref suffix in our grammar).
    let is_url = pkg.contains("://") || pkg.starts_with("git@");
    let (base, suffix) = if is_url {
        (pkg.to_string(), None)
    } else {
        match pkg.rsplit_once('@') {
            Some((b, s)) => (b.to_string(), Some(s.to_string())),
            None => (pkg.to_string(), None),
        }
    };

    let mut marketplace = marketplace_flag.map(str::to_string);
    let mut git_ref = None;
    if let Some(s) = suffix {
        if marketplace.is_none() && registry.marketplaces.contains_key(&s) {
            marketplace = Some(s); // pkg@marketplace
        } else {
            git_ref = Some(s); // pkg@ref (possibly alongside --marketplace)
        }
    }

    if let Some(mname) = marketplace {
        let entry = registry.marketplaces.get(&mname).with_context(|| {
            format!("no marketplace '{mname}' — add it with `forge plugin marketplace add {mname} <source>`")
        })?;
        Ok(Resolved {
            clone_target: entry.source.clone(),
            subdir: Some(base.clone()),
            git_ref: git_ref.or_else(|| entry.git_ref.clone()),
            name: base,
            marketplace: Some(mname),
        })
    } else {
        let name = derive_pkg_name(&base);
        Ok(Resolved {
            clone_target: base,
            subdir: None,
            git_ref,
            name,
            marketplace: None,
        })
    }
}

/// Build the git clone URL for a clone target, injecting a token for private GitHub repos.
fn clone_url(target: &str, token: Option<&str>) -> String {
    let with_token = |host_path: &str| match token {
        Some(t) => format!("https://x-access-token:{t}@{host_path}"),
        None => format!("https://{host_path}"),
    };
    if target.contains("://") || target.starts_with("git@") {
        // A full URL: inject the token into an https github URL when we have one.
        if let (Some(t), Some(rest)) = (token, target.strip_prefix("https://github.com/")) {
            return format!("https://x-access-token:{t}@github.com/{rest}");
        }
        return target.to_string();
    }
    // owner/repo shorthand → GitHub.
    let repo = target.trim_end_matches('/').trim_end_matches(".git");
    with_token(&format!("github.com/{repo}.git"))
}

// ---------------------------------------------------------------------------
// Fetch (git) + install into the skills dir
// ---------------------------------------------------------------------------

/// A best-effort temp dir removed on drop.
struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

async fn git_clone(target: &str, git_ref: Option<&str>, token: Option<&str>) -> Result<TempDir> {
    let dir = std::env::temp_dir().join(format!("forge-pkg-{}", forge_types::new_id()));
    let url = clone_url(target, token);
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(r) = git_ref {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(&url).arg(&dir);
    let out = cmd
        .output()
        .await
        .context("running `git clone` — is git installed and on PATH?")?;
    if !out.status.success() {
        // Redact the token from any echoed URL in the error.
        let stderr = String::from_utf8_lossy(&out.stderr);
        let safe = match token {
            Some(t) => stderr.replace(t, "***"),
            None => stderr.into_owned(),
        };
        anyhow::bail!("git clone failed for '{target}': {}", safe.trim());
    }
    Ok(TempDir(dir))
}

/// Choose the directory inside a clone that holds the skills: an explicit subdir, else a `skills/`
/// subdir if present, else the repo root. Rejects a `subdir` that is absolute or contains `..`
/// components, which would otherwise let a crafted package/marketplace entry escape the clone
/// directory and point `install_from` at an arbitrary path on disk.
fn pick_root(clone: &Path, subdir: Option<&str>) -> Result<PathBuf> {
    if let Some(sub) = subdir {
        let sub_path = Path::new(sub);
        if sub_path.is_absolute()
            || sub_path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            anyhow::bail!(
                "invalid package subdirectory '{sub}': must be a relative path with no '..' components"
            );
        }
        return Ok(clone.join(sub_path));
    }
    let skills = clone.join("skills");
    Ok(if skills.is_dir() {
        skills
    } else {
        clone.to_path_buf()
    })
}

/// Install every skill found in `root` (top-level `*.md` files and any directory containing a
/// `SKILL.md`) into `skills_dir`, normalizing path/binary references. `overwrite` replaces an
/// existing skill of the same name (used by update); otherwise existing ones are kept + reported.
/// Returns the names written.
fn install_from(root: &Path, skills_dir: &Path, overwrite: bool) -> Result<Vec<String>> {
    use crate::cli::commands::import::copy_dir;
    std::fs::create_dir_all(skills_dir).ok();
    let mut installed = Vec::new();
    let entries = std::fs::read_dir(root)
        .with_context(|| format!("reading package contents at {}", root.display()))?;
    for entry in entries.flatten() {
        let from = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if from.is_dir() {
            // A skill bundle: a directory with a SKILL.md.
            if !from.join("SKILL.md").is_file() {
                continue;
            }
            let dest = skills_dir.join(&name);
            if dest.exists() {
                if !overwrite {
                    continue;
                }
                std::fs::remove_dir_all(&dest).ok();
            }
            if copy_dir(&from, &dest).is_ok() {
                normalize_in_place(&dest);
                installed.push(name);
            }
        } else if from.extension().and_then(|e| e.to_str()) == Some("md") {
            let dest = skills_dir.join(&name);
            if dest.exists() && !overwrite {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&from) {
                let content = forge_skills::normalize_skill_content(
                    &raw.replace("~/.claude/", "~/.config/forge/"),
                );
                if std::fs::write(&dest, content).is_ok() {
                    installed.push(name);
                }
            }
        }
    }
    Ok(installed)
}

/// Normalize every `.md` under a freshly-copied skill directory in place.
fn normalize_in_place(dir: &Path) {
    crate::cli::commands::import::normalize_md_dir(dir);
}

// ---------------------------------------------------------------------------
// Public install / update entry points
// ---------------------------------------------------------------------------

pub(crate) async fn install_plugin(pkg: &str, marketplace_flag: Option<String>) -> Result<()> {
    let registry = load_marketplaces_at(&marketplaces_path()?)?;
    let resolved = resolve(pkg, marketplace_flag.as_deref(), &registry)?;
    let token = github_token();
    let skills_dir = skills_dir()?;

    println!(
        "fetching {} (git clone{})…",
        resolved.name,
        resolved
            .git_ref
            .as_deref()
            .map(|r| format!(" @{r}"))
            .unwrap_or_default()
    );
    let clone = git_clone(
        &resolved.clone_target,
        resolved.git_ref.as_deref(),
        token.as_deref(),
    )
    .await?;
    let root = pick_root(&clone.0, resolved.subdir.as_deref())?;
    if !root.exists() {
        anyhow::bail!(
            "package path '{}' not found in {}",
            resolved.subdir.as_deref().unwrap_or("."),
            resolved.clone_target
        );
    }
    let installed = install_from(&root, &skills_dir, false)?;
    if installed.is_empty() {
        anyhow::bail!(
            "no skills found in {} (looked for *.md files and SKILL.md directories)",
            resolved.name
        );
    }

    record_installed_at(
        &installed_path()?,
        &resolved.name,
        InstalledEntry {
            source: resolved.clone_target.clone(),
            marketplace: resolved.marketplace.clone(),
            subdir: resolved.subdir.clone(),
            git_ref: resolved.git_ref.clone(),
            files: installed.clone(),
        },
    )?;

    println!(
        "✓ installed '{}' ({} skill(s)) into {}",
        resolved.name,
        installed.len(),
        skills_dir.display()
    );
    println!("  update later with: forge plugin update {}", resolved.name);
    Ok(())
}

pub(crate) async fn update_installed(name: Option<&str>) -> Result<()> {
    let lock_path = installed_path()?;
    let lock = load_installed_at(&lock_path)?;
    if lock.skills.is_empty() {
        println!("no installed skill packs to update (install one with `forge plugin install`).");
        return Ok(());
    }
    let targets: Vec<(String, InstalledEntry)> = match name {
        Some(n) => {
            let entry =
                lock.skills.get(n).cloned().with_context(|| {
                    format!("no installed pack '{n}' — see `forge plugin list`")
                })?;
            vec![(n.to_string(), entry)]
        }
        None => lock
            .skills
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };

    let token = github_token();
    let skills_dir = skills_dir()?;
    let mut updated = 0usize;
    for (name, entry) in targets {
        println!("updating {name} from {}…", entry.source);
        let clone = match git_clone(&entry.source, entry.git_ref.as_deref(), token.as_deref()).await
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("  ✖ {name}: {e}");
                continue;
            }
        };
        let root = match pick_root(&clone.0, entry.subdir.as_deref()) {
            Ok(root) => root,
            Err(e) => {
                eprintln!("  ✖ {name}: {e}");
                continue;
            }
        };
        match install_from(&root, &skills_dir, true) {
            Ok(files) if !files.is_empty() => {
                record_installed_at(
                    &lock_path,
                    &name,
                    InstalledEntry {
                        files,
                        ..entry.clone()
                    },
                )?;
                updated += 1;
                println!("  ✓ {name} updated");
            }
            Ok(_) => eprintln!("  ✖ {name}: no skills found after re-fetch"),
            Err(e) => eprintln!("  ✖ {name}: {e}"),
        }
    }
    println!("updated {updated} pack(s).");
    Ok(())
}

/// List installed packs (lockfile) + registered marketplaces.
pub(crate) fn list_installed_and_marketplaces() -> Result<()> {
    let lock = load_installed_at(&installed_path()?)?;
    if lock.skills.is_empty() {
        println!("no skill packs installed (install one with `forge plugin install <pkg>`).");
    } else {
        println!("installed skill packs ({}):", lock.skills.len());
        for (name, entry) in &lock.skills {
            let pin = entry
                .git_ref
                .as_deref()
                .map(|r| format!(" @{r}"))
                .unwrap_or_default();
            let via = entry
                .marketplace
                .as_deref()
                .map(|m| format!(" via {m}"))
                .unwrap_or_default();
            println!(
                "  {name}  ←  {}{pin}{via}  ({} file(s))",
                entry.source,
                entry.files.len()
            );
        }
    }
    let m = load_marketplaces_at(&marketplaces_path()?)?;
    if !m.marketplaces.is_empty() {
        println!("\nmarketplaces ({}):", m.marketplaces.len());
        for (name, entry) in &m.marketplaces {
            println!("  {name}  →  {}", entry.source);
        }
    }
    Ok(())
}

/// List installed packs and reject the unsupported remote catalog query explicitly.
pub(crate) fn list_plugins(available: bool) -> Result<()> {
    if available {
        anyhow::bail!(
            "`forge plugin list --available` is not implemented; use `forge plugin marketplace list` to inspect configured sources"
        );
    }
    list_installed_and_marketplaces()
}

#[cfg(test)]
mod tests {
    use super::registry::{remove_installed_at, MarketplaceEntry};
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("forge-mkt-{}-{}", name, forge_types::new_id()))
    }

    #[test]
    fn available_listing_returns_actionable_error() {
        let error = list_plugins(true).expect_err("remote listing is not implemented");
        assert!(error.to_string().contains("plugin marketplace list"));
    }

    #[test]
    fn marketplace_add_list_remove_round_trips() {
        let path = tmp("reg").join("marketplaces.toml");
        // add two.
        assert!(!add_marketplace_at(&path, "community", "anthropics/skills", None).unwrap());
        assert!(!add_marketplace_at(
            &path,
            "internal",
            "https://git.corp/ai.git",
            Some("main".into())
        )
        .unwrap());
        let m = load_marketplaces_at(&path).unwrap();
        assert_eq!(m.marketplaces.len(), 2);
        assert_eq!(m.marketplaces["community"].source, "anthropics/skills");
        assert_eq!(m.marketplaces["internal"].git_ref.as_deref(), Some("main"));
        // overwrite returns true.
        assert!(add_marketplace_at(&path, "community", "other/repo", None).unwrap());
        assert_eq!(
            load_marketplaces_at(&path).unwrap().marketplaces["community"].source,
            "other/repo"
        );
        // remove.
        assert!(remove_marketplace_at(&path, "community").unwrap());
        assert!(!remove_marketplace_at(&path, "community").unwrap());
        assert_eq!(load_marketplaces_at(&path).unwrap().marketplaces.len(), 1);
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn install_records_a_lockfile_entry_and_update_detects_it() {
        let path = tmp("lock").join("installed-skills.toml");
        let entry = InstalledEntry {
            source: "anthropics/skills".into(),
            marketplace: Some("community".into()),
            subdir: Some("pirate-pack".into()),
            git_ref: Some("v1.2.0".into()),
            files: vec!["pirate-pack".into()],
        };
        record_installed_at(&path, "pirate-pack", entry.clone()).unwrap();
        // update's detection step: the named pack is found with its recorded source + pin.
        let lock = load_installed_at(&path).unwrap();
        assert_eq!(lock.skills.len(), 1);
        let got = lock.skills.get("pirate-pack").expect("pack recorded");
        assert_eq!(got, &entry);
        assert_eq!(got.git_ref.as_deref(), Some("v1.2.0"));
        assert_eq!(got.subdir.as_deref(), Some("pirate-pack"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn malformed_persisted_state_rejects_mutation() {
        let registry_path = tmp("malformed-registry").join("marketplaces.toml");
        std::fs::create_dir_all(registry_path.parent().unwrap()).unwrap();
        std::fs::write(&registry_path, "not = [toml").unwrap();
        let registry_before = std::fs::read_to_string(&registry_path).unwrap();
        assert!(add_marketplace_at(&registry_path, "community", "owner/repo", None).is_err());
        assert_eq!(
            std::fs::read_to_string(&registry_path).unwrap(),
            registry_before
        );

        let lock_path = tmp("malformed-installed").join("installed-skills.toml");
        std::fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
        std::fs::write(&lock_path, "not = [toml").unwrap();
        let entry = InstalledEntry {
            source: "owner/repo".into(),
            marketplace: None,
            subdir: None,
            git_ref: None,
            files: vec!["skill.md".into()],
        };
        assert!(record_installed_at(&lock_path, "pack", entry).is_err());

        std::fs::remove_dir_all(registry_path.parent().unwrap()).ok();
        std::fs::remove_dir_all(lock_path.parent().unwrap()).ok();
    }

    #[test]
    fn removal_rejects_a_malformed_lockfile() {
        let path = tmp("malformed-lock").join("installed-skills.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "this is not valid = [toml").unwrap();

        let error = remove_installed_at(&path, "pirate-pack").expect_err("malformed lock fails");
        assert!(error.to_string().contains("parsing"));
        std::fs::remove_dir_all(path.parent().unwrap()).ok();
    }

    #[test]
    fn resolve_plain_owner_repo() {
        let r = resolve("anthropics/forge-skills", None, &Marketplaces::default()).unwrap();
        assert_eq!(r.clone_target, "anthropics/forge-skills");
        assert_eq!(r.subdir, None);
        assert_eq!(r.name, "forge-skills");
        assert_eq!(r.marketplace, None);
    }

    #[test]
    fn resolve_owner_repo_with_ref() {
        let r = resolve("owner/repo@v2", None, &Marketplaces::default()).unwrap();
        assert_eq!(r.clone_target, "owner/repo");
        assert_eq!(r.git_ref.as_deref(), Some("v2"));
        assert_eq!(r.subdir, None);
    }

    #[test]
    fn resolve_pkg_at_marketplace() {
        let mut reg = Marketplaces::default();
        reg.marketplaces.insert(
            "community".into(),
            MarketplaceEntry {
                source: "anthropics/forge-marketplace".into(),
                git_ref: Some("main".into()),
            },
        );
        let r = resolve("pirate-pack@community", None, &reg).unwrap();
        assert_eq!(r.clone_target, "anthropics/forge-marketplace");
        assert_eq!(r.subdir.as_deref(), Some("pirate-pack"));
        assert_eq!(r.name, "pirate-pack");
        assert_eq!(r.marketplace.as_deref(), Some("community"));
        // marketplace ref inherited when none on the pkg.
        assert_eq!(r.git_ref.as_deref(), Some("main"));
    }

    #[test]
    fn resolve_bare_pkg_with_marketplace_flag() {
        let mut reg = Marketplaces::default();
        reg.marketplaces.insert(
            "internal".into(),
            MarketplaceEntry {
                source: "corp/skills".into(),
                git_ref: None,
            },
        );
        let r = resolve("auth-pack", Some("internal"), &reg).unwrap();
        assert_eq!(r.clone_target, "corp/skills");
        assert_eq!(r.subdir.as_deref(), Some("auth-pack"));
    }

    #[test]
    fn resolve_unknown_marketplace_errors() {
        assert!(resolve("pkg@nope", None, &Marketplaces::default())
            .map(|r| r.git_ref)
            // `nope` isn't registered → treated as a git ref, NOT an error.
            .unwrap()
            .is_some());
        // but a bare pkg with an unknown --marketplace flag IS an error.
        assert!(resolve("pkg", Some("nope"), &Marketplaces::default()).is_err());
    }

    #[test]
    fn resolve_full_url_not_at_split() {
        let r = resolve(
            "https://git.corp/team/skills.git",
            None,
            &Marketplaces::default(),
        )
        .unwrap();
        assert_eq!(r.clone_target, "https://git.corp/team/skills.git");
        assert_eq!(r.subdir, None);
        assert_eq!(r.name, "skills");
    }

    #[test]
    fn clone_url_injects_token_for_private_github() {
        assert_eq!(
            clone_url("owner/repo", Some("ghp_x")),
            "https://x-access-token:ghp_x@github.com/owner/repo.git"
        );
        assert_eq!(
            clone_url("owner/repo", None),
            "https://github.com/owner/repo.git"
        );
        assert_eq!(
            clone_url("https://github.com/owner/repo.git", Some("ghp_x")),
            "https://x-access-token:ghp_x@github.com/owner/repo.git"
        );
        // Non-GitHub URL is left untouched (token rides via the user's git credential helper).
        assert_eq!(
            clone_url("https://git.corp/team/x.git", Some("ghp_x")),
            "https://git.corp/team/x.git"
        );
    }
}
