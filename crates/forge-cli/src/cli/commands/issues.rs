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

/// One issue of a drain, as `GET /api/issues/drains` reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrainItemLine {
    pub(crate) number: u64,
    pub(crate) status: String,
    pub(crate) session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DrainLine {
    pub(crate) id: String,
    pub(crate) cwd: String,
    pub(crate) status: String,
    pub(crate) line: String,
    pub(crate) parallel: usize,
    pub(crate) meter: Option<String>,
    pub(crate) paused_reason: Option<String>,
    pub(crate) items: Vec<DrainItemLine>,
}

impl DrainLine {
    pub(crate) fn of(value: &Value) -> Self {
        Self {
            id: value["id"].as_str().unwrap_or_default().to_string(),
            cwd: value["cwd"].as_str().unwrap_or_default().to_string(),
            status: value["status"].as_str().unwrap_or_default().to_string(),
            line: value["line"].as_str().unwrap_or_default().to_string(),
            parallel: usize::try_from(value["parallel"].as_u64().unwrap_or(0)).unwrap_or(0),
            meter: value["envelope"]["meter"].as_str().map(str::to_string),
            paused_reason: value["paused_reason"].as_str().map(str::to_string),
            items: value["items"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|i| DrainItemLine {
                    number: i["number"].as_u64().unwrap_or(0),
                    status: i["status"].as_str().unwrap_or_default().to_string(),
                    session_id: i["session_id"].as_str().map(str::to_string),
                })
                .collect(),
        }
    }
}

/// What a `drain` request asks for.
#[derive(Debug, Clone)]
pub(crate) struct DrainRequest {
    pub(crate) numbers: Vec<u64>,
    pub(crate) parallel: Option<usize>,
    pub(crate) budget: Option<String>,
    pub(crate) worktree: bool,
    pub(crate) mode: String,
    pub(crate) method: Option<String>,
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

impl Daemon {
    pub(crate) async fn drain(&self, cwd: &str, request: &DrainRequest) -> Result<DrainLine> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/issues/drain", self.base, self.token);
        let mut body = json!({
            "cwd": cwd,
            "numbers": request.numbers,
            "worktree": request.worktree,
            "mode": request.mode,
        });
        if let Some(parallel) = request.parallel {
            body["parallel"] = json!(parallel);
        }
        if let Some(budget) = &request.budget {
            body["budget"] = json!(budget);
        }
        if let Some(method) = &request.method {
            body["method"] = json!(method);
        }
        Ok(DrainLine::of(&self.call(http.post(url).json(&body)).await?))
    }

    /// Every drain, newest first.
    pub(crate) async fn drains(&self) -> Result<Vec<DrainLine>> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/issues/drains", self.base, self.token);
        let value = self.call(http.get(url)).await?;
        Ok(value["drains"]
            .as_array()
            .into_iter()
            .flatten()
            .map(DrainLine::of)
            .collect())
    }

    pub(crate) async fn cancel_drain(&self, id: &str) -> Result<DrainLine> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/issues/drain/cancel", self.base, self.token);
        let value = self.call(http.post(url).json(&json!({"id": id}))).await?;
        Ok(DrainLine::of(&value))
    }

    /// `POST /api/sessions/{id}/ship`: the daemon's answer, refusals included (`ok: false`).
    pub(crate) async fn ship(&self, session: &str, body: &Value) -> Result<Value> {
        let http = local_http_client(&self.base);
        let url = format!("{}/{}/api/sessions/{session}/ship", self.base, self.token);
        // Commit hooks, a push and `gh` each run on the host; the client default is too short.
        self.call(
            http.post(url)
                .timeout(std::time::Duration::from_secs(180))
                .json(body),
        )
        .await
    }

    /// Full ids of the sessions this daemon is running, for resolving a typed prefix.
    pub(crate) async fn running_sessions(&self) -> Result<Vec<crate::attach::SessionInfo>> {
        let http = local_http_client(&self.base);
        crate::attach::fetch_sessions(&http, &self.base, &self.token).await
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

/// Everything `forge issues` was given.
pub(crate) struct IssuesOpts {
    pub(crate) start: Option<u64>,
    pub(crate) no_worktree: bool,
    pub(crate) watch: String,
    pub(crate) method: Option<String>,
    pub(crate) state: String,
    pub(crate) drain: Vec<u64>,
    pub(crate) parallel: Option<usize>,
    pub(crate) budget: Option<String>,
    pub(crate) drain_status: bool,
    pub(crate) cancel_drain: Option<String>,
    pub(crate) url: Option<String>,
    pub(crate) token: Option<String>,
}

pub(crate) fn print_drain(drain: &DrainLine) {
    println!("{}  [{}]  {}", drain.line, drain.status, drain.id);
    if let Some(meter) = &drain.meter {
        println!("  {meter}");
    }
    if let Some(reason) = &drain.paused_reason {
        println!("  {reason}");
    }
    for item in &drain.items {
        let session = item
            .session_id
            .as_deref()
            .map_or_else(String::new, |s| format!("  forge attach {s}"));
        println!("  #{:<5} {}{session}", item.number, item.status);
    }
}

pub(crate) async fn issues_cmd(opts: IssuesOpts) -> Result<()> {
    let IssuesOpts {
        start,
        no_worktree,
        watch,
        method,
        state,
        drain,
        parallel,
        budget,
        drain_status,
        cancel_drain,
        url,
        token,
    } = opts;
    let daemon = Daemon::resolve(url, token)?;
    let cwd = current_dir();
    if let Some(id) = cancel_drain {
        let cancelled = daemon.cancel_drain(&id).await?;
        println!("drain cancelled; sessions already running keep going");
        print_drain(&cancelled);
        return Ok(());
    }
    if drain_status {
        let drains = daemon.drains().await?;
        if drains.is_empty() {
            println!("no drains");
        }
        for d in &drains {
            print_drain(d);
        }
        return Ok(());
    }
    if !drain.is_empty() {
        let started = daemon
            .drain(
                &cwd,
                &DrainRequest {
                    numbers: drain,
                    parallel,
                    budget,
                    worktree: !no_worktree,
                    mode: watch,
                    method,
                },
            )
            .await?;
        println!(
            "draining {} issues, {} at a time",
            started.items.len(),
            started.parallel
        );
        print_drain(&started);
        println!("\nfollow it with: forge issues --drain-status   stop it with: forge issues --cancel-drain {}", started.id);
        return Ok(());
    }
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
    println!("\nstart one with: forge issues --start <number>, or several with: forge issues --drain 12,15,19");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drain_row_reads_its_items_and_meter() {
        let value = json!({
            "id": "drain-1", "status": "paused", "parallel": 2,
            "line": "Drain: 2 started, 1 queued (paused)",
            "paused_reason": "Envelope spent",
            "envelope": {"meter": "envelope 30/30% claude weekly"},
            "items": [
                {"number": 12, "status": "pr_open", "session_id": "s1"},
                {"number": 15, "status": "queued", "session_id": null},
            ],
        });
        let line = DrainLine::of(&value);
        assert_eq!(line.parallel, 2);
        assert_eq!(line.meter.as_deref(), Some("envelope 30/30% claude weekly"));
        assert_eq!(line.items[0].session_id.as_deref(), Some("s1"));
        assert_eq!(line.items[1].session_id, None);
    }

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
