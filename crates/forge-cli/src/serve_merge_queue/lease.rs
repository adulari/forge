//! The cross-daemon lease: one daemon at a time lands a repository's PRs. See
//! docs/rfcs/shared-merge-queue.md.
//!
//! The lease is a ref in the repository itself, `refs/forge/merge-queue/<base>`, that points at
//! a parentless chain of commits whose messages carry the owner. GitHub makes it atomic: creating
//! a ref that exists and moving a ref to a commit that is not a fast-forward both fail with 422,
//! so two daemons that read the same state cannot both take the lease.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::serve_pr::{Gh, GhError};

/// A holder renews at least this often (the daemon's PR poll is 30 s).
pub(crate) const RENEW_SECS: i64 = 60;
/// A lease nobody renewed for this long is stale and may be taken over, so a dead daemon never
/// blocks the queue for long.
pub(crate) const STALE_SECS: i64 = 600;
/// After GitHub refuses the lease, the daemon lands on its own for this long before asking again.
const DEGRADED_SECS: i64 = 300;
const CONTENDED_RETRIES: usize = 3;
const MARKER: &str = "forge-merge-queue-lease";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    /// Stable across restarts, so a restarted daemon renews its own lease instead of waiting it out.
    pub(crate) id: String,
    pub(crate) host: String,
}

impl Identity {
    pub(crate) fn new(host: &str, data_dir: &Path) -> Self {
        let host = clean(host);
        let mut hash: u32 = 0x811c_9dc5;
        for byte in data_dir.display().to_string().bytes() {
            hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
        }
        Self {
            id: format!("{host}-{hash:08x}"),
            host,
        }
    }

    pub(crate) fn of_this_daemon() -> Self {
        let dir = forge_config::data_dir().unwrap_or_default();
        Self::new(&crate::anywhere::default_host_name(), &dir)
    }
}

fn clean(text: &str) -> String {
    let out: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    if out.is_empty() {
        "unknown".into()
    } else {
        out
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    /// This daemon holds the lease.
    Mine,
    /// Another daemon holds it.
    Other,
    /// GitHub refused the lease; this daemon lands on its own.
    Unavailable,
}

/// What a surface shows about a repository's lease.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub(crate) struct LeaseView {
    pub(crate) status: Status,
    /// Host name of the daemon that holds it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) holder: Option<String>,
    /// The PR the holder is landing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pr: Option<u64>,
    /// When the lease goes stale unless renewed (unix seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<i64>,
    /// When this daemon last looked (unix seconds).
    pub(crate) seen_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

impl LeaseView {
    /// `landing on ssu-dev: PR #123`
    pub(crate) fn landing(&self) -> String {
        let pr = self.pr.map_or(String::new(), |n| format!(": PR #{n}"));
        match self.status {
            Status::Mine => format!("landing here{pr}"),
            Status::Other => format!(
                "landing on {}{pr}",
                self.holder.as_deref().unwrap_or("another daemon")
            ),
            Status::Unavailable => self
                .note
                .clone()
                .unwrap_or_else(|| "shared queue unavailable".into()),
        }
    }
}

/// A repository's lease as every surface lists it: the facts, and the one line they read as.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub(crate) struct LeaseRow {
    pub(crate) status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) holder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pr: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<i64>,
    /// `landing on ssu-dev: PR #123`, `landing here: PR #123` or why the shared queue is off.
    pub(crate) text: String,
}

impl From<&LeaseView> for LeaseRow {
    fn from(v: &LeaseView) -> Self {
        Self {
            status: v.status,
            holder: v.holder.clone(),
            pr: v.pr,
            expires_at: v.expires_at,
            text: v.landing(),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Lease {
    /// The daemon may update, verify and land.
    Held(LeaseView),
    /// Another daemon is landing; hold.
    Busy(LeaseView),
    /// GitHub refused the lease: land on this daemon's own, with the warning.
    Degraded(LeaseView),
    /// Nothing to coordinate (no GitHub origin).
    Off,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Body {
    owner: String,
    host: String,
    pr: Option<u64>,
    renewed_at: i64,
    released: bool,
}

impl Body {
    fn encode(&self) -> String {
        format!(
            "{MARKER} v1 owner={} host={} pr={} renewed={} released={}",
            self.owner,
            self.host,
            self.pr.map_or_else(|| "-".into(), |n| n.to_string()),
            self.renewed_at,
            u8::from(self.released)
        )
    }

    fn decode(text: &str) -> Option<Self> {
        let mut words = text.split_whitespace();
        if words.next()? != MARKER || words.next()? != "v1" {
            return None;
        }
        let field: HashMap<&str, &str> = words.filter_map(|w| w.split_once('=')).collect();
        Some(Self {
            owner: (*field.get("owner")?).to_string(),
            host: (*field.get("host")?).to_string(),
            pr: field.get("pr").and_then(|p| p.parse().ok()),
            renewed_at: field.get("renewed")?.parse().ok()?,
            released: *field.get("released")? == "1",
        })
    }

    fn stale(&self, now: i64) -> bool {
        now - self.renewed_at > STALE_SECS
    }

    fn view(&self, mine: bool, now: i64) -> LeaseView {
        LeaseView {
            status: if mine { Status::Mine } else { Status::Other },
            holder: Some(self.host.clone()),
            pr: self.pr,
            expires_at: Some(self.renewed_at + STALE_SECS),
            seen_at: now,
            note: None,
        }
    }
}

#[derive(Debug)]
enum Fail {
    /// 422 on a ref write: another daemon moved first.
    Contended,
    Unavailable(String),
}

struct Held {
    root: PathBuf,
    slug: String,
    refname: String,
    sha: String,
    body: Body,
    renewed_local: i64,
}

#[derive(Default)]
struct State {
    held: HashMap<String, Held>,
    degraded_until: HashMap<String, i64>,
    bodies: HashMap<String, Body>,
}

/// One daemon's view of the leases it holds. A daemon owns exactly one; the tests make two.
pub(crate) struct Leases {
    identity: Identity,
    slug_override: Option<String>,
    state: Mutex<State>,
}

impl Leases {
    pub(crate) fn new(identity: Identity) -> Self {
        Self {
            identity,
            slug_override: None,
            state: Mutex::new(State::default()),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(host: &str, slug: &str) -> Self {
        Self {
            identity: Identity::new(host, Path::new(host)),
            slug_override: Some(slug.to_string()),
            state: Mutex::new(State::default()),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take or renew the lease for `repo` (the queue's key) as its head lands `pr`.
    pub(crate) async fn acquire(
        &self,
        gh: &Gh,
        root: &Path,
        repo: &str,
        base: &str,
        pr: Option<u64>,
        now: i64,
    ) -> Lease {
        let Some(slug) = self
            .slug_override
            .clone()
            .or_else(|| github_slug(&origin_url(root)?))
        else {
            return Lease::Off;
        };
        {
            let state = self.state();
            if state.degraded_until.get(repo).is_some_and(|t| now < *t) {
                return Lease::Degraded(unavailable(
                    "shared queue unavailable, landing on its own",
                    now,
                ));
            }
            if let Some(held) = state.held.get(repo) {
                if now - held.renewed_local < RENEW_SECS && held.body.pr == pr {
                    return Lease::Held(held.body.view(true, now));
                }
            }
        }
        let refname = format!("refs/forge/merge-queue/{}", ref_leaf(base));
        let mut last = String::new();
        for _ in 0..CONTENDED_RETRIES {
            match self
                .attempt(gh, root, repo, &slug, &refname, base, pr, now)
                .await
            {
                Ok(lease) => return lease,
                Err(Fail::Contended) => {}
                Err(Fail::Unavailable(why)) => {
                    last = why;
                    break;
                }
            }
        }
        let why = if last.is_empty() {
            "the lease kept changing hands".to_string()
        } else {
            last
        };
        tracing::warn!(repo, %why, "shared merge queue unavailable; landing on this daemon's own");
        let mut state = self.state();
        state.held.remove(repo);
        state
            .degraded_until
            .insert(repo.to_string(), now + DEGRADED_SECS);
        Lease::Degraded(unavailable(
            &format!(
                "shared queue unavailable ({}), landing on its own",
                short(&why)
            ),
            now,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn attempt(
        &self,
        gh: &Gh,
        root: &Path,
        repo: &str,
        slug: &str,
        refname: &str,
        base: &str,
        pr: Option<u64>,
        now: i64,
    ) -> Result<Lease, Fail> {
        let me = &self.identity;
        let current = self.read(gh, root, slug, refname).await?;
        let parent = match &current {
            None => None,
            Some((sha, body)) => {
                let takeable = body.owner == me.id || body.released || body.stale(now);
                if !takeable {
                    self.state().held.remove(repo);
                    return Ok(Lease::Busy(body.view(false, now)));
                }
                Some(sha.clone())
            }
        };
        let body = Body {
            owner: me.id.clone(),
            host: me.host.clone(),
            pr,
            renewed_at: now,
            released: false,
        };
        let sha = self
            .write(gh, root, slug, refname, base, &body, parent.as_deref())
            .await?;
        let mut view = body.view(true, now);
        if let Some((_, old)) = &current {
            if old.owner != me.id && !old.released {
                view.note = Some(format!("took over a stale lease from {}", old.host));
            }
        }
        self.state().held.insert(
            repo.to_string(),
            Held {
                root: root.to_path_buf(),
                slug: slug.to_string(),
                refname: refname.to_string(),
                sha,
                body,
                renewed_local: now,
            },
        );
        Ok(Lease::Held(view))
    }

    async fn read(
        &self,
        gh: &Gh,
        root: &Path,
        slug: &str,
        refname: &str,
    ) -> Result<Option<(String, Body)>, Fail> {
        let path = format!(
            "repos/{slug}/git/ref/{}",
            refname.trim_start_matches("refs/")
        );
        let sha = match gh.run(root, &["api", &path], None).await {
            Ok(out) => json_str(&out, &["object", "sha"])
                .ok_or_else(|| Fail::Unavailable("unreadable ref answer".into()))?,
            Err(GhError::Failed(m)) if m.contains("HTTP 404") || m.contains("Not Found") => {
                return Ok(None)
            }
            Err(e) => return Err(Fail::Unavailable(e.message())),
        };
        if let Some(body) = self.state().bodies.get(&sha) {
            return Ok(Some((sha, body.clone())));
        }
        let out = gh
            .run(
                root,
                &["api", &format!("repos/{slug}/git/commits/{sha}")],
                None,
            )
            .await
            .map_err(|e| Fail::Unavailable(e.message()))?;
        let body = json_str(&out, &["message"])
            .and_then(|m| Body::decode(&m))
            .ok_or_else(|| Fail::Unavailable("unreadable lease".into()))?;
        let mut state = self.state();
        if state.bodies.len() > 64 {
            state.bodies.clear();
        }
        state.bodies.insert(sha.clone(), body.clone());
        Ok(Some((sha, body)))
    }

    /// Commit `body` on top of `parent` and move the ref there; a ref write that loses the race
    /// is [`Fail::Contended`].
    #[allow(clippy::too_many_arguments)]
    async fn write(
        &self,
        gh: &Gh,
        root: &Path,
        slug: &str,
        refname: &str,
        base: &str,
        body: &Body,
        parent: Option<&str>,
    ) -> Result<String, Fail> {
        let tree = local_tree(root, base).ok_or_else(|| {
            Fail::Unavailable(format!("no local origin/{base} to take a tree from"))
        })?;
        let message = format!("message={}", body.encode());
        let tree_arg = format!("tree={tree}");
        let parent_arg = parent.map(|p| format!("parents[]={p}"));
        let commits = format!("repos/{slug}/git/commits");
        let mut args = vec![
            "api", &commits, "-X", "POST", "-f", &message, "-f", &tree_arg,
        ];
        if let Some(p) = &parent_arg {
            args.extend(["-f", p]);
        }
        let out = gh
            .run(root, &args, None)
            .await
            .map_err(|e| Fail::Unavailable(e.message()))?;
        let sha = json_str(&out, &["sha"])
            .ok_or_else(|| Fail::Unavailable("unreadable commit answer".into()))?;
        let sha_arg = format!("sha={sha}");
        let result = if parent.is_some() {
            let path = format!("repos/{slug}/git/{refname}");
            gh.run(
                root,
                &[
                    "api",
                    &path,
                    "-X",
                    "PATCH",
                    "-f",
                    &sha_arg,
                    "-F",
                    "force=false",
                ],
                None,
            )
            .await
        } else {
            let ref_arg = format!("ref={refname}");
            let path = format!("repos/{slug}/git/refs");
            gh.run(
                root,
                &["api", &path, "-X", "POST", "-f", &ref_arg, "-f", &sha_arg],
                None,
            )
            .await
        };
        match result {
            Ok(_) => Ok(sha),
            Err(GhError::Failed(m)) if m.contains("HTTP 422") => Err(Fail::Contended),
            Err(e) => Err(Fail::Unavailable(e.message())),
        }
    }

    /// Give the lease back so the other daemon does not wait for it to go stale. Best effort: a
    /// failure leaves the lease to expire.
    pub(crate) async fn release(&self, gh: &Gh, repo: &str, now: i64) {
        let Some(held) = self.state().held.remove(repo) else {
            return;
        };
        let body = Body {
            released: true,
            renewed_at: now,
            pr: None,
            ..held.body.clone()
        };
        if let Err(why) = self
            .write(
                gh,
                &held.root,
                &held.slug,
                &held.refname,
                &base_of(repo),
                &body,
                Some(&held.sha),
            )
            .await
        {
            tracing::warn!(
                repo,
                ?why,
                "could not release the merge queue lease; it expires on its own"
            );
        }
    }

    /// Release the leases of repositories where this daemon has nothing left to land.
    pub(crate) async fn release_idle(&self, gh: &Gh, busy: impl Fn(&str) -> bool, now: i64) {
        let idle: Vec<String> = self
            .state()
            .held
            .keys()
            .filter(|repo| !busy(repo))
            .cloned()
            .collect();
        for repo in idle {
            self.release(gh, &repo, now).await;
        }
    }

    #[cfg(test)]
    pub(crate) fn holds(&self, repo: &str) -> bool {
        self.state().held.contains_key(repo)
    }
}

fn unavailable(note: &str, now: i64) -> LeaseView {
    LeaseView {
        status: Status::Unavailable,
        holder: None,
        pr: None,
        expires_at: None,
        seen_at: now,
        note: Some(note.to_string()),
    }
}

fn short(why: &str) -> String {
    let line = why.lines().next().unwrap_or("").trim();
    line.chars().take(80).collect()
}

/// The queue key is `origin @ base`.
fn base_of(repo: &str) -> String {
    repo.rsplit_once(" @ ")
        .map_or_else(|| "main".into(), |(_, b)| b.to_string())
}

/// A base branch as one ref path segment, so `release/1.0` and `release` cannot collide as a
/// directory and a file.
fn ref_leaf(base: &str) -> String {
    base.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn json_str(text: &str, path: &[&str]) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(text).ok()?;
    for key in path {
        value = value.get_mut(*key)?.take();
    }
    value.as_str().map(str::to_string)
}

pub(super) fn origin_url(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `owner/repo` of a github.com remote (https, ssh or scp-style), else `None`.
pub(crate) fn github_slug(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let (owner, name) = rest.split_once('/')?;
    let valid = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    };
    (valid(owner) && valid(name)).then(|| format!("{owner}/{name}"))
}

/// A tree the repository is known to hold: the commit objects of the lease need one.
fn local_tree(root: &Path, base: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "rev-parse",
            "--verify",
            "-q",
            &format!("origin/{base}^{{tree}}"),
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let tree = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!tree.is_empty()).then_some(tree)
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn slugs_come_from_github_remotes_only() {
        for url in [
            "https://github.com/adulari/forge.git",
            "git@github.com:adulari/forge.git",
            "ssh://git@github.com/adulari/forge",
            "https://github.com/adulari/forge/",
        ] {
            assert_eq!(github_slug(url).as_deref(), Some("adulari/forge"), "{url}");
        }
        for url in [
            "/tmp/remote.git",
            "https://gitlab.com/a/b.git",
            "https://github.com/onlyowner",
            "https://github.com/a b/c",
        ] {
            assert_eq!(github_slug(url), None, "{url}");
        }
    }

    #[test]
    fn a_lease_body_round_trips_and_garbage_is_refused() {
        let body = Body {
            owner: "ssu-dev-0a1b2c3d".into(),
            host: "ssu-dev".into(),
            pr: Some(123),
            renewed_at: 1_700_000_000,
            released: false,
        };
        assert_eq!(Body::decode(&body.encode()), Some(body.clone()));
        let idle = Body {
            pr: None,
            released: true,
            ..body
        };
        assert_eq!(Body::decode(&idle.encode()), Some(idle));
        assert_eq!(Body::decode("seed"), None);
        assert_eq!(Body::decode("forge-merge-queue-lease v2 owner=a"), None);
    }

    #[test]
    fn identities_are_stable_per_host_and_store_and_refs_are_one_segment() {
        let a = Identity::new("ssu dev", Path::new("/data/a"));
        assert_eq!(a, Identity::new("ssu dev", Path::new("/data/a")));
        assert_ne!(a.id, Identity::new("ssu dev", Path::new("/data/b")).id);
        assert_eq!(a.host, "ssu-dev");
        assert_eq!(ref_leaf("release/1.0"), "release_1.0");
        assert_eq!(base_of("https://x/y.git @ release/1.0"), "release/1.0");
    }
}

#[cfg(test)]
#[path = "lease_tests.rs"]
mod tests;
