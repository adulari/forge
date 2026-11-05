use serde::Deserialize;

pub struct Api {
    base: String,
}

#[derive(Deserialize, Clone)]
pub struct Session {
    pub id: String,
    pub title: Option<String>,
    pub cwd: String,
    pub busy: bool,
    pub waiting: bool,
    pub diff_added: Option<i64>,
    pub diff_removed: Option<i64>,
    pub last_activity: Option<i64>,
    pub worktree: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub adds: i64,
    pub dels: i64,
}

#[derive(Deserialize, Clone)]
pub struct AgentChanges {
    pub session_id: String,
    pub title: String,
    pub waiting: bool,
    pub busy: bool,
    pub files: Vec<ChangedFile>,
}

#[derive(Deserialize)]
struct ChangesResponse {
    sessions: Vec<AgentChanges>,
}

#[derive(Deserialize)]
struct Hunk {
    header: String,
    lines: Vec<String>,
}

#[derive(Deserialize)]
struct DiffFile {
    path: String,
    hunks: Vec<Hunk>,
}

#[derive(Deserialize)]
struct DiffResponse {
    files: Vec<DiffFile>,
}

#[derive(Deserialize, Clone)]
pub struct HistoryRow {
    pub seq: i64,
    pub role: String,
    pub content: String,
}

impl Api {
    pub fn from_env() -> Self {
        if let Ok(base) = std::env::var("FORGE_BASE") {
            return Self { base };
        }
        let dir = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap()));
        let state: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{dir}/forge/serve-state.json")).expect("serve-state.json (point XDG_CONFIG_HOME at the mock daemon config)")).unwrap();
        Self { base: state["base_url"].as_str().unwrap().to_string() }
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        Ok(ureq::get(&format!("{}{}", self.base, path)).call()?.into_json()?)
    }

    pub fn sessions(&self) -> anyhow::Result<Vec<Session>> {
        self.get("/api/sessions")
    }

    pub fn changes(&self, cwd: &str) -> anyhow::Result<Vec<AgentChanges>> {
        Ok(self.get::<ChangesResponse>(&format!("/api/projects/changes?cwd={cwd}"))?.sessions)
    }

    /// The daemon's structured hunks, rebuilt as the unified patch the diff widget parses.
    pub fn diff_patch(&self, session: &str, path: &str) -> anyhow::Result<String> {
        let response: DiffResponse = self.get(&format!("/api/git/diff?session={session}&path={path}&context=3"))?;
        let mut patch = String::new();
        for file in response.files {
            patch.push_str(&format!("diff --git a/{0} b/{0}\n--- a/{0}\n+++ b/{0}\n", file.path));
            for hunk in file.hunks {
                patch.push_str(&fix_header(&hunk.header, &hunk.lines));
                patch.push('\n');
                for line in hunk.lines {
                    patch.push_str(&line);
                    patch.push('\n');
                }
            }
        }
        Ok(patch)
    }

    /// The daemon pages history newest-first, 200 rows at a time.
    pub fn history(&self, session: &str) -> anyhow::Result<Vec<HistoryRow>> {
        let mut all: Vec<HistoryRow> = Vec::new();
        let mut before: Option<i64> = None;
        loop {
            let mut path = format!("/api/history?session={session}&limit=200");
            if let Some(b) = before {
                path.push_str(&format!("&before={b}"));
            }
            let page: Vec<HistoryRow> = self.get(&path)?;
            if page.is_empty() {
                break;
            }
            before = page.iter().map(|r| r.seq).min();
            all.extend(page);
        }
        all.sort_by_key(|r| r.seq);
        Ok(all)
    }
}

/// The daemon caps a diff at 4,000 lines and can cut the last hunk short; recount so the patch parser accepts it.
fn fix_header(header: &str, lines: &[String]) -> String {
    let old = lines.iter().filter(|l| l.starts_with(' ') || l.starts_with('-')).count();
    let new = lines.iter().filter(|l| l.starts_with(' ') || l.starts_with('+')).count();
    let mut parts = header.splitn(4, ' ');
    let (_, o, n) = (parts.next(), parts.next().unwrap_or("-1"), parts.next().unwrap_or("+1"));
    let start = |s: &str| s.trim_start_matches(['-', '+']).split(',').next().unwrap_or("1").to_string();
    let rest = header.splitn(2, " @@").nth(1).unwrap_or("");
    format!("@@ -{},{} +{},{} @@{}", start(o), old, start(n), new, rest)
}
