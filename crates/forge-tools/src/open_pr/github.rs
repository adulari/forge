//! The GitHub side of `open_pr`: which repository a git remote points at, and whether the signed-in
//! `gh` user may push to it.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Repo {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl Repo {
    /// The `[HOST/]OWNER/REPO` form `gh --repo` takes.
    pub fn slug(&self) -> String {
        if self.host == "github.com" {
            format!("{}/{}", self.owner, self.name)
        } else {
            format!("{}/{}/{}", self.host, self.owner, self.name)
        }
    }

    /// A link that renders `path` at `commit` inline in a PR body, private repos included (the
    /// viewer's own repo access authorizes it).
    pub fn raw_url(&self, commit: &str, path: &str) -> String {
        format!(
            "https://{}/{}/{}/blob/{commit}/{path}?raw=true",
            self.host, self.owner, self.name
        )
    }
}

/// Parse a git remote URL: `git@host:owner/repo(.git)`, `ssh://git@host[:port]/owner/repo`,
/// `https://[user@]host/owner/repo(.git)`, `git://host/owner/repo`.
pub(super) fn parse_remote_url(url: &str) -> Option<Repo> {
    let url = url.trim();
    let (host, path) = if let Some((_, rest)) = url.split_once("://") {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let host = host.split_once(':').map_or(host, |(h, _)| h);
        (host, path)
    } else {
        let (authority, path) = url.split_once(':')?;
        let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        (host, path)
    };
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let (owner, name) = path.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    if host.is_empty() || !valid(owner) || !valid(name) {
        return None;
    }
    let host = host
        .strip_prefix("www.")
        .unwrap_or(host)
        .to_ascii_lowercase();
    Some(Repo {
        host,
        owner: owner.to_string(),
        name: name.to_string(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RepoInfo {
    pub permission: String,
    pub default_branch: String,
    pub archived: bool,
}

/// `gh repo view --json viewerPermission,defaultBranchRef,isArchived`.
pub(super) fn parse_repo_view(json: &str) -> Result<RepoInfo, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| format!("unexpected `gh repo view` output ({e}): {json}"))?;
    Ok(RepoInfo {
        permission: value
            .get("viewerPermission")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        default_branch: value
            .pointer("/defaultBranchRef/name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        archived: value
            .get("isArchived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Whether a `viewerPermission` allows pushing branches.
pub(super) fn can_push(permission: &str) -> bool {
    matches!(permission, "ADMIN" | "MAINTAIN" | "WRITE")
}

/// The URL of the first PR in a `gh pr list --json url` array.
pub(super) fn first_pr_url(json: &str) -> Option<String> {
    let value: Value = serde_json::from_str(json).ok()?;
    value
        .as_array()?
        .first()?
        .get("url")?
        .as_str()
        .map(str::to_string)
}

/// The PR URL `gh pr create` prints as its last line.
pub(super) fn created_pr_url(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| line.starts_with("https://"))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_url_forms_resolve_to_the_same_repo() {
        let expected = Repo {
            host: "github.com".into(),
            owner: "acme".into(),
            name: "shop.web".into(),
        };
        for url in [
            "git@github.com:acme/shop.web.git",
            "git@github.com:acme/shop.web",
            "ssh://git@github.com/acme/shop.web.git",
            "ssh://git@github.com:22/acme/shop.web",
            "https://github.com/acme/shop.web.git",
            "https://token@github.com/acme/shop.web/",
            "https://www.github.com/acme/shop.web",
            "git://github.com/acme/shop.web.git",
        ] {
            assert_eq!(parse_remote_url(url), Some(expected.clone()), "{url}");
        }
        assert_eq!(expected.slug(), "acme/shop.web");
    }

    #[test]
    fn enterprise_hosts_keep_their_host_in_the_slug() {
        let repo = parse_remote_url("https://ghe.corp.example/team/app").unwrap();
        assert_eq!(repo.slug(), "ghe.corp.example/team/app");
        assert_eq!(
            repo.raw_url("abc", "s/1.png"),
            "https://ghe.corp.example/team/app/blob/abc/s/1.png?raw=true"
        );
    }

    #[test]
    fn local_paths_and_odd_urls_are_not_repos() {
        for url in [
            "/srv/git/repo.git",
            "../other",
            "https://github.com/onlyowner",
            "",
        ] {
            assert_eq!(parse_remote_url(url), None, "{url}");
        }
    }

    #[test]
    fn only_write_level_permissions_may_push() {
        for p in ["ADMIN", "MAINTAIN", "WRITE"] {
            assert!(can_push(p));
        }
        for p in ["TRIAGE", "READ", "", "write"] {
            assert!(!can_push(p), "{p}");
        }
    }

    #[test]
    fn gh_outputs_parse() {
        let info = parse_repo_view(
            r#"{"viewerPermission":"WRITE","defaultBranchRef":{"name":"main"},"isArchived":false}"#,
        )
        .unwrap();
        assert_eq!(info.permission, "WRITE");
        assert_eq!(info.default_branch, "main");
        assert!(!info.archived);
        assert!(parse_repo_view("not json").is_err());
        assert_eq!(
            first_pr_url(r#"[{"url":"https://github.com/a/b/pull/7"}]"#).as_deref(),
            Some("https://github.com/a/b/pull/7")
        );
        assert_eq!(first_pr_url("[]"), None);
        assert_eq!(
            created_pr_url("Creating pull request…\nhttps://github.com/a/b/pull/8\n").as_deref(),
            Some("https://github.com/a/b/pull/8")
        );
    }
}
