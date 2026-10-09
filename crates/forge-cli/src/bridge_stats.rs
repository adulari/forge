/// Read live usage stats from local Codex and Claude session files.
///
/// Codex: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` — each turn emits
/// an `event_msg / token_count` line with rate-limit windows. The primary/secondary positions
/// are not semantic: current Codex can emit only a weekly window as `primary`, so windows are
/// identified by their `window_minutes` value (300 = 5h, 10080 = weekly).
///
/// Claude: `~/.claude/projects/**/*.jsonl` — each assistant turn has
/// `message.usage.{input,output,cache_read,cache_creation}_tokens`.
/// Claude doesn't embed rate-limit percentages, so we return raw token sums.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{Datelike, Local};
use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct BridgeStats {
    pub codex_5h_pct: Option<f64>,
    pub codex_weekly_pct: Option<f64>,
    /// Exact `rate_limits.plan_type` from the same Codex rollout observation as the quota.
    /// It is account-authoritative when fresh and supersedes a stale OAuth JWT claim.
    pub codex_plan: Option<String>,
    pub codex_plan_observed_at: Option<i64>,
    /// When `codex_5h_pct` was actually OBSERVED (epoch secs): the rollout line's own `timestamp`
    /// field, falling back to the file's mtime. Rollout files can be hours old, so seeding the
    /// store with `now()` would let this stale reading mask fresher `x-codex-*` header data in
    /// the shared codex quota bucket — seed with this instead (`Store::record_quota_at`).
    pub codex_5h_observed_at: Option<i64>,
    /// When `codex_weekly_pct` was actually observed (see `codex_5h_observed_at`).
    pub codex_weekly_observed_at: Option<i64>,
    pub claude_5h_pct: Option<f64>,
    pub claude_weekly_pct: Option<f64>,
    /// Actual cache observation times. These must accompany cache-derived values into the shared
    /// store so a 674-hour-old statusline file cannot overwrite a newer live-header probe.
    pub claude_5h_observed_at: Option<i64>,
    pub claude_weekly_observed_at: Option<i64>,
    pub claude_5h_in: u64,
    pub claude_5h_out: u64,
    pub claude_weekly_in: u64,
    pub claude_weekly_out: u64,
    /// Age (seconds) of the Claude rate-limit cache when it was read — `None` if the cache is
    /// missing. Lets the overlay flag stale percentages instead of presenting them as live.
    pub claude_rl_age_secs: Option<i64>,
}

/// Environment this probe must NOT hand to its child.
///
/// THE PROBE HAS POISONED THE USER'S STORE THREE TIMES. `claude --print` loads the project's
/// `.mcp.json` from its working directory, which names a `forge` binary; that grandchild inherits
/// whatever `FORGE_DB` we were started with and migrates the shared store to ITS schema. Afterwards
/// the installed release binary cannot open the store at all (`SchemaTooNew`), so the Anywhere
/// connector dies while local Forge keeps answering on its pre-migration connection and looks
/// healthy. Observed 2026-07-17 (v17 → v21) and twice on 2026-08-06 (24 → 25); each recovery needed
/// a manual `PRAGMA user_version` write.
///
/// The store path is the one that corrupts state, but any variable that steers a Forge child is
/// wrong to leak into a quota probe: a checkpoint id would attribute unrelated work, a sink path
/// would inject lifecycle events into a session this probe has nothing to do with, and a permission
/// mode would silently widen what that child may do.
const PROBE_SCRUBBED_ENV: [&str; 5] = [
    "FORGE_DB",
    "FORGE_SUBAGENT_SINK",
    "FORGE_CHECKPOINT_SESSION",
    "FORGE_CHECKPOINT_SEQ",
    "FORGE_PERMISSION_MODE",
];

/// Build the probe child. Split out so the isolation below is assertable without a `claude` binary
/// on the machine running the tests.
fn probe_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("claude");
    cmd.args([
        "--debug",
        "--print",
        "--model",
        "haiku",
        "--append-system-prompt",
        "Reply with a single period.",
    ])
    .arg(".")
    .env("ANTHROPIC_LOG", "debug")
    .stdin(std::process::Stdio::null());
    for key in PROBE_SCRUBBED_ENV {
        cmd.env_remove(key);
    }
    // A quota probe has no business in the project directory. Running there makes `claude` discover
    // the project's `.mcp.json` and boot its whole MCP tree — the mechanism behind the store
    // poisoning above, and separately a startup stall this repo already hit (mesh.bridge_mcp_external
    // exists because of it). Credentials come from the home directory, not the cwd, so the probe
    // works identically from a neutral one.
    cmd.current_dir(std::env::temp_dir());
    cmd
}

/// Harvest the CURRENT Claude rate-limit utilisation for BOTH windows by running one minimal
/// `claude` turn with `--debug` and reading the `anthropic-ratelimit-unified-{5h,7d}-utilization`
/// response headers it logs. Unlike the stream-json `rate_limit_event` (which only reports the
/// window near its limit), the headers always carry both the 5-hour and 7-day windows — the same
/// data Claude Code feeds its statusline. The only fresh source when the statusline cache is stale.
/// Returns `(window, fraction, reset instant)` tuples. Best-effort: empty on failure. Costs one
/// tiny Haiku turn, so callers should gate it on staleness.
pub fn probe_claude_limits() -> Vec<(String, f64, Option<i64>)> {
    // Bound the probe: `claude --print` can stall on a cold network or an auth prompt. Run it on a
    // detached thread and wait at most PROBE_TIMEOUT for the result; on timeout return empty so the
    // (backgrounded) quota refresh completes instead of leaking a task blocked on a hung child. The
    // statusline cache / next refresh fills the numbers in later.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        #[allow(unused_mut)]
        let mut cmd = probe_command();
        // `--debug` makes the real `claude` CLI write verbose diagnostic output straight to the
        // controlling terminal via /dev/tty, bypassing stdout/stderr redirection entirely (a
        // common "always show this even if piped" pattern). Stdio::piped() (what `.output()`
        // uses) does NOT stop that — it only redirects fds 1/2, and /dev/tty is a separate path
        // to the same terminal as long as this child shares our session. Detach it into its own
        // session (setsid) so /dev/tty has no controlling terminal to resolve to: the probe still
        // runs and its captured stdout/stderr are unaffected, but it can no longer scribble raw
        // debug text over our own TUI's rendering on the same pty. Unix-only; Windows consoles
        // don't have this controlling-terminal/setsid concept, so no equivalent is needed there.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Safety: setsid() is async-signal-safe and valid to call between fork and exec
            // (the same pattern already used in forge-tools/src/shell.rs's sandbox pre_exec).
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        let out = cmd.output();
        let _ = tx.send(out);
    });
    let out = match rx.recv_timeout(PROBE_TIMEOUT) {
        Ok(Ok(out)) => out,
        _ => return Vec::new(),
    };
    // Debug logs (with the headers) go to stderr; scan both streams to be safe.
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    let mut res = Vec::new();
    for (utilization_header, reset_header, window) in [
        (
            "anthropic-ratelimit-unified-5h-utilization",
            "anthropic-ratelimit-unified-5h-reset",
            "five_hour",
        ),
        (
            "anthropic-ratelimit-unified-7d-utilization",
            "anthropic-ratelimit-unified-7d-reset",
            "weekly",
        ),
    ] {
        if let Some(frac) = first_float_after(&text, utilization_header) {
            res.push((
                window.to_string(),
                frac,
                first_reset_after(&text, reset_header),
            ));
        }
    }
    res
}

/// Read the active Codex OAuth account's authoritative, account-wide quota headers. The Codex
/// backend has no quota-only endpoint, so this is one tiny `gpt-5.4-mini` request with a
/// one-character reply. Callers gate it on freshness; an unavailable OAuth session intentionally
/// yields no observation so the CLI-bridge rollout fallback remains available.
pub async fn probe_codex_limits() -> Vec<forge_types::QuotaHint> {
    if !forge_provider::has_codex_oauth_session() {
        return Vec::new();
    }
    forge_provider::probe_codex_quota()
        .await
        .unwrap_or_default()
}

/// Find the first numeric run (digits + `.`) appearing after `key` in `text`. Tolerant of the
/// surrounding `": "..."` / log punctuation between the key and its value.
fn first_float_after(text: &str, key: &str) -> Option<f64> {
    let after = &text[text.find(key)? + key.len()..];
    let start = after.find(|c: char| c.is_ascii_digit())?;
    let tail = &after[start..];
    let end = tail
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(tail.len());
    tail[..end].parse().ok()
}

fn first_reset_after(text: &str, key: &str) -> Option<i64> {
    let after = &text[text.find(key)? + key.len()..];
    after
        .split_whitespace()
        .take(8)
        .map(|token| token.trim_matches(|c: char| "\"',;[]{}()".contains(c)))
        .find_map(|token| {
            token
                .parse::<i64>()
                .ok()
                .filter(|value| *value > 1_000_000_000)
                .or_else(|| {
                    chrono::DateTime::parse_from_rfc3339(token)
                        .ok()
                        .map(|timestamp| timestamp.timestamp())
                })
        })
}

pub fn fetch() -> BridgeStats {
    let mut stats = BridgeStats::default();
    if let Ok(home) = std::env::var("HOME") {
        let home = PathBuf::from(home);
        fetch_codex(&mut stats, &home);
        fetch_claude(&mut stats, &home);
    }
    stats
}

// ── Codex ────────────────────────────────────────────────────────────────────

fn fetch_codex(stats: &mut BridgeStats, home: &Path) {
    let root = home.join(".codex/sessions");
    // Collect all session files from the last 2 days, sorted newest-first.
    let files = jsonl_files_in_recent_days(&root, 2);
    let now = now_epoch();
    for path in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines().rev() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if v["type"] != "event_msg" || v["payload"]["type"] != "token_count" {
                continue;
            }
            let observed_at = codex_line_observed_at(&v, &path);
            let rl = &v["payload"]["rate_limits"];
            if !forge_provider::codex_rollout_is_account_wide(rl) {
                continue;
            }
            if let Some(plan) = rl["plan_type"]
                .as_str()
                .filter(|plan| !plan.trim().is_empty())
            {
                stats.codex_plan = Some(plan.trim().to_string());
                stats.codex_plan_observed_at = observed_at;
            }
            for key in ["primary", "secondary"] {
                let window = &rl[key];
                let resets_at = window["resets_at"].as_i64().unwrap_or(0);
                match window["window_minutes"].as_i64() {
                    Some(300) if resets_at > now => {
                        stats.codex_5h_pct = window["used_percent"].as_f64();
                        stats.codex_5h_observed_at = observed_at;
                    }
                    Some(300) if resets_at > 0 && now - resets_at < 5 * 3600 => {
                        // A recently reset 5h window was known empty only at the reset instant.
                        // Stamp the inference there so a later real OAuth observation wins.
                        stats.codex_5h_pct = Some(0.0);
                        stats.codex_5h_observed_at = Some(resets_at);
                    }
                    Some(10080) if resets_at > now => {
                        stats.codex_weekly_pct = window["used_percent"].as_f64();
                        stats.codex_weekly_observed_at = observed_at;
                    }
                    // Do not infer a window from its primary/secondary position. In particular,
                    // an absent 5h limit must remain absent rather than appear as 0% or 27%.
                    _ => {}
                }
            }
            // Stop as soon as we have at least weekly (most durable) data.
            if stats.codex_weekly_pct.is_some() {
                return;
            }
            break; // No valid data in this file; try the next one.
        }
    }
}

/// When a rollout line's reading was actually observed: the line's own top-level `timestamp`
/// (ISO-8601, written by codex on every event), falling back to the file's mtime — codex wrote
/// the file at observation time, so mtime is a faithful (if slightly late) stand-in.
fn codex_line_observed_at(v: &Value, path: &Path) -> Option<i64> {
    if let Some(ts) = v["timestamp"].as_str().map(parse_ts).filter(|&t| t > 0) {
        return Some(ts);
    }
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(|t| {
            t.duration_since(UNIX_EPOCH)
                .unwrap_or(Duration::ZERO)
                .as_secs() as i64
        })
}

/// All Codex session `.jsonl` files from the last `look_back` days, sorted newest-first.
fn jsonl_files_in_recent_days(root: &Path, look_back: u32) -> Vec<PathBuf> {
    let now = Local::now();
    let mut all: Vec<PathBuf> = Vec::new();
    for delta in 0..=look_back {
        let day = now.date_naive() - chrono::Duration::days(delta as i64);
        let dir = root
            .join(day.year().to_string())
            .join(format!("{:02}", day.month()))
            .join(format!("{:02}", day.day()));
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
                .collect();
            // A resumed Codex session keeps its original rollout filename but receives new
            // rate-limit events for hours afterwards. Filename order therefore picks a stale
            // short-lived session over the actively-written long-lived one; mtime is the only
            // correct freshness ordering here. Path order is just a deterministic tie-breaker.
            files.sort_by(|a, b| {
                let modified = |path: &PathBuf| {
                    std::fs::metadata(path)
                        .and_then(|meta| meta.modified())
                        .unwrap_or(UNIX_EPOCH)
                };
                modified(b).cmp(&modified(a)).then_with(|| b.cmp(a))
            });
            all.extend(files);
        }
    }
    all
}

// ── Claude ───────────────────────────────────────────────────────────────────

fn fetch_claude_rate_limits(stats: &mut BridgeStats, home: &Path) {
    let path = home.join(".claude/.rate-limits-cache.json");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&content) else {
        return;
    };
    // Staleness is per-window: a 5-hour window's % is meaningless once it's hours old, but a
    // 7-day window barely moves — keeping a 6–24h-old weekly reading is far better than showing
    // nothing (which makes the overlay fall back to raw tokens and the mesh see the plan as 0%).
    // The cache only refreshes while Claude Code renders its statusline, so it routinely lags.
    let observed_at = v["ts"].as_i64().filter(|&ts| ts > 0);
    let age = now_epoch().saturating_sub(observed_at.unwrap_or(0));
    stats.claude_rl_age_secs = Some(age);
    if age <= 6 * 3600 {
        stats.claude_5h_pct = v["5h_pct"].as_f64();
        stats.claude_5h_observed_at = observed_at;
    }
    if age <= 24 * 3600 {
        stats.claude_weekly_pct = v["7d_pct"].as_f64();
        stats.claude_weekly_observed_at = observed_at;
    }
}

/// Token totals for one minute of a Claude transcript. Bucketing keeps the per-file aggregate small
/// while leaving the 5h and 7d cutoffs accurate to the minute.
type MinuteBucket = (u64, u64);

/// What the incremental scan remembers about one transcript.
///
/// `~/.claude/projects` holds gigabytes of append-only JSONL touched within the week. Re-reading all
/// of it on every start burned ~47% of a core for 10–20 s; with this index an unchanged file costs
/// one `stat` and a grown one costs only the bytes appended since `offset`.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
struct FileScan {
    /// Bytes already folded into `buckets`: always the end of a complete line.
    offset: u64,
    /// Modification time (secs) when `offset` was recorded; lets a same-size rewrite be noticed.
    mtime: i64,
    /// Minute (epoch secs / 60) → (input, output) tokens.
    buckets: std::collections::BTreeMap<i64, MinuteBucket>,
}

type ScanIndex = std::collections::HashMap<String, FileScan>;

fn scan_index_path(home: &Path) -> PathBuf {
    home.join(".cache/forge/claude-usage-index.json")
}

fn load_scan_index(home: &Path) -> ScanIndex {
    std::fs::read(scan_index_path(home))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_scan_index(home: &Path, index: &ScanIndex) {
    let path = scan_index_path(home);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(bytes) = serde_json::to_vec(index) else {
        return;
    };
    // Write-then-rename so a concurrent Forge never reads a half-written index.
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, bytes).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

fn bytes_contain(hay: &[u8], needle: &[u8]) -> bool {
    let Some((&first, rest)) = needle.split_first() else {
        return true;
    };
    let mut i = 0;
    while let Some(pos) = hay[i..].iter().position(|&b| b == first) {
        let start = i + pos;
        if hay[start + 1..].starts_with(rest) {
            return true;
        }
        i = start + 1;
    }
    false
}

/// Fold the lines appended to `path` since `scan.offset` into `scan.buckets`.
fn scan_claude_file(path: &Path, len: u64, mtime: i64, scan: &mut FileScan) {
    use std::io::{BufRead, Seek, SeekFrom};
    // Shrunk or rewritten in place: the old aggregate no longer describes this file.
    if len < scan.offset || (len == scan.offset && mtime != scan.mtime && scan.offset != 0) {
        *scan = FileScan::default();
    }
    scan.mtime = mtime;
    if len == scan.offset {
        return;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    if file.seek(SeekFrom::Start(scan.offset)).is_err() {
        return;
    }
    let mut reader = std::io::BufReader::with_capacity(256 * 1024, file);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // A partial trailing line is still being written; pick it up next time.
                if line.last() != Some(&b'\n') {
                    break;
                }
                scan.offset += n as u64;
            }
        }
        // User/tool-result lines carry the large payloads and no usage: skip them unparsed.
        if !bytes_contain(&line, b"\"type\":\"assistant\"") {
            continue;
        }
        let Ok(v) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        if v["type"] != "assistant" {
            continue;
        }
        let ts = v["timestamp"].as_str().map(parse_ts).unwrap_or(0);
        if ts <= 0 {
            continue;
        }
        let u = &v["message"]["usage"];
        let inp = u["input_tokens"].as_u64().unwrap_or(0)
            + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
            + u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
        let out = u["output_tokens"].as_u64().unwrap_or(0);
        let bucket = scan.buckets.entry(ts.div_euclid(60)).or_default();
        bucket.0 += inp;
        bucket.1 += out;
    }
}

fn fetch_claude(stats: &mut BridgeStats, home: &Path) {
    fetch_claude_rate_limits(stats, home);
    let root = home.join(".claude/projects");
    let now_secs = now_epoch();
    let cutoff_5h = now_secs - 5 * 3600;
    let cutoff_week = now_secs - 7 * 24 * 3600;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_recent_jsonl(&root, cutoff_week, &mut files);

    let mut old_index = load_scan_index(home);
    let mut index = ScanIndex::new();
    let mut changed = false;
    for path in files {
        let key = path.to_string_lossy().into_owned();
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);
        let mut scan = old_index.remove(&key).unwrap_or_default();
        let before = scan.offset;
        scan_claude_file(&path, meta.len(), mtime, &mut scan);
        changed |= scan.offset != before;
        let weekly_floor = cutoff_week.div_euclid(60);
        let (week_in, week_out, h5_in, h5_out) = scan.buckets.range(weekly_floor..).fold(
            (0, 0, 0, 0),
            |(wi, wo, hi, ho), (&minute, &(i, o))| {
                let recent = minute * 60 >= cutoff_5h;
                (
                    wi + i,
                    wo + o,
                    hi + if recent { i } else { 0 },
                    ho + if recent { o } else { 0 },
                )
            },
        );
        stats.claude_weekly_in += week_in;
        stats.claude_weekly_out += week_out;
        stats.claude_5h_in += h5_in;
        stats.claude_5h_out += h5_out;
        // Buckets older than the window can never count again.
        scan.buckets = scan.buckets.split_off(&weekly_floor);
        index.insert(key, scan);
    }
    // Files that fell out of the window were dropped from `index`; persist that too.
    changed |= !old_index.is_empty();
    if changed {
        save_scan_index(home, &index);
    }
}

fn collect_recent_jsonl(dir: &PathBuf, cutoff_secs: i64, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recent_jsonl(&path, cutoff_secs, out);
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            let recent = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .unwrap_or(Duration::ZERO)
                        .as_secs() as i64
                        >= cutoff_secs
                })
                .unwrap_or(false);
            if recent {
                out.push(path);
            }
        }
    }
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as i64
}

fn parse_ts(s: &str) -> i64 {
    s.parse::<chrono::DateTime<chrono::Utc>>()
        .map(|d| d.timestamp())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a rollout file under `<home>/.codex/sessions/<today>/` and return its path.
    fn write_named_rollout(home: &Path, name: &str, lines: &str) -> PathBuf {
        let now = Local::now();
        let dir = home
            .join(".codex/sessions")
            .join(now.year().to_string())
            .join(format!("{:02}", now.month()))
            .join(format!("{:02}", now.day()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, lines).unwrap();
        path
    }

    fn write_rollout(home: &Path, lines: &str) -> PathBuf {
        write_named_rollout(home, "rollout-test.jsonl", lines)
    }

    fn token_count_line(timestamp: Option<&str>, p_resets: i64, s_resets: i64) -> String {
        let mut v = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {"used_percent": 12.0, "window_minutes": 300, "resets_at": p_resets},
                    "secondary": {"used_percent": 3.0, "window_minutes": 10080, "resets_at": s_resets},
                }
            }
        });
        if let Some(ts) = timestamp {
            v["timestamp"] = serde_json::json!(ts);
        }
        v.to_string()
    }

    #[test]
    fn fetch_codex_observed_at_comes_from_the_line_timestamp() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        // A line whose event timestamp is 2 hours old — observed_at must reflect THAT, not now.
        let two_hours_ago = chrono::DateTime::from_timestamp(now - 7200, 0)
            .unwrap()
            .to_rfc3339();
        write_rollout(
            home.path(),
            &token_count_line(Some(&two_hours_ago), now + 3600, now + 86400),
        );

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        assert_eq!(stats.codex_5h_pct, Some(12.0));
        assert_eq!(stats.codex_weekly_pct, Some(3.0));
        assert_eq!(stats.codex_5h_observed_at, Some(now - 7200));
        assert_eq!(stats.codex_weekly_observed_at, Some(now - 7200));
    }

    #[test]
    fn fetch_codex_reset_inference_is_stamped_at_the_reset_instant() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        // Primary window reset 30 minutes ago; secondary still open. The inferred 0% is only
        // known true AT the reset instant — stamping it later would let it clobber real
        // post-reset readings (the 21:50-beats-21:37 live failure).
        let reset_at = now - 1800;
        write_rollout(home.path(), &token_count_line(None, reset_at, now + 86400));

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        assert_eq!(stats.codex_5h_pct, Some(0.0), "reset window infers 0%");
        assert_eq!(
            stats.codex_5h_observed_at,
            Some(reset_at),
            "the inference is knowledge as of the reset instant, not now"
        );
    }

    #[test]
    fn fetch_codex_weekly_only_primary_does_not_fabricate_a_five_hour_window() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        let line = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {"used_percent": 27.0, "window_minutes": 10080, "resets_at": now + 86400},
                    "secondary": null,
                }
            }
        })
        .to_string();
        write_rollout(home.path(), &line);

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        assert_eq!(stats.codex_5h_pct, None);
        assert_eq!(stats.codex_weekly_pct, Some(27.0));
    }

    #[test]
    fn fetch_codex_ignores_model_specific_limit_snapshots() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        // The official CLI can append a fresh zero-percent *model-specific* limit (for example
        // `codex_bengalfox`) after an older account-wide `codex` observation. It must not replace
        // the ChatGPT account's weekly allowance used by Mesh.
        let account = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "limit_id": "codex",
                    "primary": {"used_percent": 31.0, "window_minutes": 10080, "resets_at": now + 86400},
                    "secondary": null,
                }
            }
        });
        let model = serde_json::json!({
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "limit_id": "codex_bengalfox",
                    "limit_name": "GPT-5.3-Codex-Spark",
                    "primary": {"used_percent": 0.0, "window_minutes": 10080, "resets_at": now + 86400},
                    "secondary": null,
                }
            }
        });
        write_named_rollout(home.path(), "rollout-account.jsonl", &account.to_string());
        write_named_rollout(home.path(), "rollout-model.jsonl", &model.to_string());

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        assert_eq!(stats.codex_weekly_pct, Some(31.0));
    }

    #[test]
    fn fetch_codex_rollout_plan_is_preserved_verbatim_with_its_observation_time() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        let line = serde_json::json!({
            "timestamp": chrono::DateTime::from_timestamp(now - 30, 0).unwrap().to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "rate_limits": {
                    "primary": {"used_percent": 27.0, "window_minutes": 10080, "resets_at": now + 86400},
                    "secondary": null,
                    "plan_type": "pro",
                }
            }
        })
        .to_string();
        write_rollout(home.path(), &line);

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        assert_eq!(stats.codex_plan.as_deref(), Some("pro"));
        assert_eq!(stats.codex_plan_observed_at, Some(now - 30));
    }

    #[test]
    fn claude_cache_keeps_its_true_observation_time_for_store_staleness() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        let dir = home.path().join(".claude");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(".rate-limits-cache.json"),
            serde_json::json!({"ts": now - 120, "5h_pct": 14.0, "7d_pct": 48.0}).to_string(),
        )
        .unwrap();

        let mut stats = BridgeStats::default();
        fetch_claude_rate_limits(&mut stats, home.path());
        assert_eq!(stats.claude_5h_pct, Some(14.0));
        assert_eq!(stats.claude_weekly_pct, Some(48.0));
        assert_eq!(stats.claude_5h_observed_at, Some(now - 120));
        assert_eq!(stats.claude_weekly_observed_at, Some(now - 120));
    }

    #[test]
    fn fetch_codex_observed_at_falls_back_to_file_mtime() {
        let home = tempfile::tempdir().unwrap();
        let now = now_epoch();
        // No timestamp field on the line — mtime (the write above, ~now) stands in.
        write_rollout(
            home.path(),
            &token_count_line(None, now + 3600, now + 86400),
        );

        let mut stats = BridgeStats::default();
        fetch_codex(&mut stats, home.path());
        let observed = stats
            .codex_5h_observed_at
            .expect("mtime fallback must supply an observation time");
        assert!(
            (observed - now).abs() <= 5,
            "mtime of a just-written file should be ~now (got {observed}, now {now})"
        );
    }

    /// The probe must not hand a Forge child the environment that lets it migrate the shared store.
    /// Asserting on the built Command (rather than spawning) keeps this runnable without `claude`
    /// installed, and pins the exact variables — a new one added to PROBE_SCRUBBED_ENV without a
    /// reason is easier to notice than a silently missing removal.
    #[test]
    fn probe_child_gets_no_store_or_session_environment() {
        let cmd = super::probe_command();
        let removed: Vec<&str> = cmd
            .get_envs()
            .filter(|(_, v)| v.is_none())
            .filter_map(|(k, _)| k.to_str())
            .collect();
        for key in super::PROBE_SCRUBBED_ENV {
            assert!(
                removed.contains(&key),
                "{key} must be removed from the probe child's environment, got {removed:?}"
            );
        }
    }

    /// A probe running in the project directory is what made `claude` load the project's .mcp.json
    /// and spawn the forge grandchild that poisoned the store.
    #[test]
    fn probe_child_runs_outside_the_project_directory() {
        let cmd = super::probe_command();
        let cwd = cmd.get_current_dir().expect("probe must pin a cwd");
        assert_eq!(
            cwd,
            std::env::temp_dir(),
            "probe must run from a neutral cwd"
        );
        assert_ne!(
            cwd,
            std::env::current_dir().unwrap().as_path(),
            "probe must not inherit the project cwd"
        );
    }

    fn claude_line(secs_ago: i64, inp: u64, out: u64) -> String {
        let ts = chrono::DateTime::from_timestamp(now_epoch() - secs_ago, 0)
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"usage":{{"input_tokens":{inp},"output_tokens":{out}}}}}}}"#
        ) + "\n"
    }

    fn claude_project(home: &Path) -> PathBuf {
        let dir = home.join(".claude/projects/p");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.jsonl")
    }

    #[test]
    fn claude_totals_are_incremental_and_survive_a_restart() {
        let home = tempfile::tempdir().unwrap();
        let file = claude_project(home.path());
        let user_line = "{\"type\":\"user\",\"message\":\"hello\"}\n";
        std::fs::write(
            &file,
            format!(
                "{user_line}{}{}",
                claude_line(60, 10, 1),
                claude_line(6 * 3600, 100, 5)
            ),
        )
        .unwrap();

        let mut stats = BridgeStats::default();
        fetch_claude(&mut stats, home.path());
        assert_eq!((stats.claude_5h_in, stats.claude_5h_out), (10, 1));
        assert_eq!((stats.claude_weekly_in, stats.claude_weekly_out), (110, 6));

        let index = load_scan_index(home.path());
        let len = std::fs::metadata(&file).unwrap().len();
        assert_eq!(
            index.values().next().unwrap().offset,
            len,
            "index covers the file"
        );

        // Append one complete line and one still being written: only the complete one counts,
        // and the old lines are not re-added.
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&file)
            .unwrap();
        write!(f, "{}{{\"type\":\"assist", claude_line(30, 1, 2)).unwrap();
        drop(f);
        let mut stats = BridgeStats::default();
        fetch_claude(&mut stats, home.path());
        assert_eq!((stats.claude_5h_in, stats.claude_5h_out), (11, 3));
        assert_eq!((stats.claude_weekly_in, stats.claude_weekly_out), (111, 8));
    }

    #[test]
    fn claude_totals_rescan_a_truncated_transcript() {
        let home = tempfile::tempdir().unwrap();
        let file = claude_project(home.path());
        std::fs::write(
            &file,
            format!("{}{}", claude_line(60, 10, 1), claude_line(120, 20, 2)),
        )
        .unwrap();
        fetch_claude(&mut BridgeStats::default(), home.path());

        std::fs::write(&file, claude_line(60, 7, 3)).unwrap();
        let mut stats = BridgeStats::default();
        fetch_claude(&mut stats, home.path());
        assert_eq!((stats.claude_weekly_in, stats.claude_weekly_out), (7, 3));
    }
}
