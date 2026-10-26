//! Minimal GitHub REST client for the report pipeline: create / read / update an issue and add a
//! comment. The token comes from the environment, is held in a [`Secret`] whose `Debug` is
//! redacted, and is only ever placed in the `authorization` header.

use std::cmp::Ordering;
use std::fmt;
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct Secret(String);

impl Secret {
    pub(crate) fn new(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

pub(crate) struct GitHub {
    http: reqwest::Client,
    api_base: String,
    repo: String,
    token: Secret,
}

impl fmt::Debug for GitHub {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHub")
            .field("api_base", &self.api_base)
            .field("repo", &self.repo)
            .field("token", &self.token)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct IssueState {
    pub(crate) open: bool,
}

impl GitHub {
    pub(crate) fn new(api_base: &str, repo: &str, token: Secret) -> anyhow::Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
            api_base: api_base.trim_end_matches('/').to_string(),
            repo: repo.to_string(),
            token,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(
                method,
                format!("{}/repos/{}{path}", self.api_base, self.repo),
            )
            .header("authorization", format!("Bearer {}", self.token.0))
            .header("accept", "application/vnd.github+json")
            .header("x-github-api-version", "2022-11-28")
            .header("user-agent", "forge-relay")
    }

    async fn send(
        &self,
        builder: reqwest::RequestBuilder,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<(u16, serde_json::Value)> {
        let builder = match body {
            Some(body) => builder
                .header("content-type", "application/json")
                .body(body.to_string()),
            None => builder,
        };
        let response = builder.send().await?;
        let status = response.status().as_u16();
        let text = response.text().await.unwrap_or_default();
        Ok((status, serde_json::from_str(&text).unwrap_or_default()))
    }

    pub(crate) async fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[&str],
    ) -> anyhow::Result<u64> {
        let (status, json) = self
            .send(
                self.request(reqwest::Method::POST, "/issues"),
                Some(serde_json::json!({"title": title, "body": body, "labels": labels})),
            )
            .await?;
        anyhow::ensure!(
            status == 201 || status == 200,
            "github create issue: HTTP {status}"
        );
        json["number"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("github create issue: no issue number"))
    }

    /// `None` when the issue no longer exists.
    pub(crate) async fn get_issue(&self, number: u64) -> anyhow::Result<Option<IssueState>> {
        let (status, json) = self
            .send(
                self.request(reqwest::Method::GET, &format!("/issues/{number}")),
                None,
            )
            .await?;
        match status {
            200 => Ok(Some(IssueState {
                open: json["state"].as_str() != Some("closed"),
            })),
            404 | 410 => Ok(None),
            other => anyhow::bail!("github get issue: HTTP {other}"),
        }
    }

    pub(crate) async fn update_issue(
        &self,
        number: u64,
        body: &str,
        reopen: bool,
    ) -> anyhow::Result<()> {
        let mut payload = serde_json::json!({ "body": body });
        if reopen {
            payload["state"] = "open".into();
        }
        let (status, _) = self
            .send(
                self.request(reqwest::Method::PATCH, &format!("/issues/{number}")),
                Some(payload),
            )
            .await?;
        anyhow::ensure!(status == 200, "github update issue: HTTP {status}");
        Ok(())
    }

    pub(crate) async fn comment(&self, number: u64, text: &str) -> anyhow::Result<()> {
        let (status, _) = self
            .send(
                self.request(reqwest::Method::POST, &format!("/issues/{number}/comments")),
                Some(serde_json::json!({ "body": text })),
            )
            .await?;
        anyhow::ensure!(status == 201, "github comment: HTTP {status}");
        Ok(())
    }
}

/// `(major, minor, patch, is_release)`; a pre-release sorts below its release.
fn version_key(v: &str) -> (u64, u64, u64, bool) {
    let v = v.trim_start_matches('v');
    let (core, pre) = match v.split_once(['-', '+']) {
        Some((core, _)) => (core, true),
        None => (v, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        !pre,
    )
}

pub(crate) fn version_cmp(a: &str, b: &str) -> Ordering {
    version_key(a).cmp(&version_key(b))
}

pub(crate) fn version_newer(a: &str, than: &str) -> bool {
    version_cmp(a, than) == Ordering::Greater
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(version_newer("2.10.0", "2.9.9"));
        assert!(!version_newer("2.9.9", "2.10.0"));
        assert!(!version_newer("2.23.6", "2.23.6"));
        assert!(version_newer("2.23.6", "2.23.6-rc1"));
    }

    #[test]
    fn debug_never_shows_the_token() {
        let gh = GitHub::new("http://x", "o/r", Secret::new("ghp_supersecret123".into())).unwrap();
        assert!(!format!("{gh:?}").contains("supersecret"));
    }
}
