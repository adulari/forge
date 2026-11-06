//! `forge issues [--start N]`: list the current project's GitHub issues, or start a worktree
//! session on one with PR Watch armed. Talks to the daemon (`/api/issues*`), which runs `gh`
//! and owns the session; shared with the TUI's `/issues` overlay.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use crate::attach::{local_http_client, resolve_base_url, resolve_token};

/// One row of `GET /api/issues`, as the terminal shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IssueLine {
    pub(crate) number: u64,
    pub(crate) title: String,
    pub(crate) labels: Vec<String>,
    pub(crate) author: String,
    pub(crate) comments: usize,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Started {
    pub(crate) session_id: String,
    pub(crate) worktree: Option<String>,
    pub(crate) watch: Option<String>,
}

pub(crate) struct Daemon {
    base: String,
    token: String,
}

impl Daemon {
    pub(crate) fn resolve(url: Option<String>, token: Option<String>) -> Result<Self> {
        Ok(Self {
            base: resolve_base_url(url),
            token: resolve_token(token)?,
        })
    }

    async fn call(&self, request: reqwest::RequestBuilder) -> Result<Value> {
        let response = request.send().await.map_err(|e| {
            anyhow!(
                "could not reach the forge serve daemon at {} - is it running? \
                 (start it with `forge serve --local`)  [{e}]",
                self.base
            )
        })?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
        if !status.is_success() {
            let detail = value
                .get("error")
                .and_then(Value::as_str)
                .map_or(body, str::to_string);
            bail!("{detail}");
        }
        Ok(value)
    }

    /// The project's issues; `Err` carries the "gh not installed / not signed in" message too.
    pub(crate) async fn list(&self, cwd: &str, state: &str) -> Result<Vec<IssueLine>> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/issues", self.base, self.token);
        let value = self
            .call(http.get(url).query(&[("cwd", cwd), ("state", state)]))
            .await?;
        if value["available"].as_bool() == Some(false) {
            bail!(
                "{}",
                value["error"].as_str().unwrap_or("GitHub is not available")
            );
        }
        Ok(value["issues"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| IssueLine {
                number: i["number"].as_u64().unwrap_or(0),
                title: i["title"].as_str().unwrap_or_default().to_string(),
                labels: i["labels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l["name"].as_str().map(str::to_string))
                    .collect(),
                author: i["author"].as_str().unwrap_or_default().to_string(),
                comments: usize::try_from(i["comment_count"].as_u64().unwrap_or(0)).unwrap_or(0),
                updated_at: i["updated_at"].as_str().unwrap_or_default().to_string(),
            })
            .collect())
    }

    pub(crate) async fn start(
        &self,
        cwd: &str,
        number: u64,
        worktree: bool,
        mode: &str,
        method: Option<&str>,
    ) -> Result<Started> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/issues/start", self.base, self.token);
        let mut body = json!({"cwd": cwd, "number": number, "worktree": worktree, "mode": mode});
        if let Some(method) = method {
            body["method"] = json!(method);
        }
        let value = self.call(http.post(url).json(&body)).await?;
        Ok(Started {
            session_id: value["id"].as_str().unwrap_or_default().to_string(),
            worktree: value["worktree"].as_str().map(str::to_string),
            watch: value["watch"]["mode"].as_str().map(str::to_string),
        })
    }
}

pub(crate) fn current_dir() -> String {
    std::env::current_dir()
        .ok()
        .or_else(forge_config::home_dir)
        .map_or_else(|| "/".into(), |d| d.display().to_string())
}

pub(crate) fn format_line(issue: &IssueLine) -> String {
    let labels = if issue.labels.is_empty() {
        String::new()
    } else {
        format!(" [{}]", issue.labels.join(", "))
    };
    let comments = match issue.comments {
        0 => String::new(),
        n => format!("  {n} comment{}", if n == 1 { "" } else { "s" }),
    };
    format!(
        "#{:<5} {}{labels}  {}{comments}",
        issue.number, issue.title, issue.author
    )
}

pub(crate) async fn issues_cmd(
    start: Option<u64>,
    no_worktree: bool,
    watch: String,
    method: Option<String>,
    state: String,
    url: Option<String>,
    token: Option<String>,
) -> Result<()> {
    let daemon = Daemon::resolve(url, token)?;
    let cwd = current_dir();
    if let Some(number) = start {
        let started = daemon
            .start(&cwd, number, !no_worktree, &watch, method.as_deref())
            .await?;
        println!("started session {} on issue #{number}", started.session_id);
        if let Some(worktree) = started.worktree {
            println!("  worktree: {worktree}");
        }
        match started.watch {
            Some(mode) => println!("  PR Watch: {mode}"),
            None => println!("  PR Watch: off"),
        }
        println!("  follow along: forge attach {}", started.session_id);
        return Ok(());
    }
    let issues = daemon.list(&cwd, &state).await?;
    if issues.is_empty() {
        println!("no {state} issues");
        return Ok(());
    }
    for issue in &issues {
        println!("{}", format_line(issue));
    }
    println!("\nstart one with: forge issues --start <number>");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_show_labels_and_comment_counts() {
        let line = IssueLine {
            number: 12,
            title: "Crash on resume".into(),
            labels: vec!["auto-report".into(), "bug".into()],
            author: "forge-bot".into(),
            comments: 2,
            updated_at: String::new(),
        };
        assert_eq!(
            format_line(&line),
            "#12    Crash on resume [auto-report, bug]  forge-bot  2 comments"
        );
        let quiet = IssueLine {
            labels: Vec::new(),
            comments: 0,
            ..line
        };
        assert_eq!(format_line(&quiet), "#12    Crash on resume  forge-bot");
    }
}
