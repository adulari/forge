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

/// Harvest the CURRENT Claude rate-limit utilisation for BOTH windows by running one minimal
/// `claude` turn with `--debug` and reading the `anthropic-ratelimit-unified-{5h,7d}-utilization`
/// response headers it logs. Unlike the stream-json `rate_limit_event` (which only reports the
/// window near its limit), the headers always carry both the 5-hour and 7-day windows — the same
/// data Claude Code feeds its statusline. The only fresh source when the statusline cache is stale.
/// Returns (window, fraction) pairs, e.g. `[("five_hour", 0.10), ("weekly", 0.81)]`. Best-effort:
/// empty on failure. Costs one tiny Haiku turn, so callers should gate it on staleness.
pub fn probe_claude_limits() -> Vec<(String, f64)> {
    // Bound the probe: `claude --print` can stall on a cold network or an auth prompt. Run it on a
    // detached thread and wait at most PROBE_TIMEOUT for the result; on timeout return empty so the
    // (backgrounded) quota refresh completes instead of leaking a task blocked on a hung child. The
    // statusline cache / next refresh fills the numbers in later.
    const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        #[allow(unused_mut)]
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
    for (hdr, window) in [
        ("anthropic-ratelimit-unified-5h-utilization", "five_hour"),
        ("anthropic-ratelimit-unified-7d-utilization", "weekly"),
    ] {
        if let Some(frac) = first_float_after(&text, hdr) {
            res.push((window.to_string(), frac));
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

fn fetch_claude(stats: &mut BridgeStats, home: &Path) {
    fetch_claude_rate_limits(stats, home);
    let root = home.join(".claude/projects");
    let now_secs = now_epoch();
    let cutoff_5h = now_secs - 5 * 3600;
    let cutoff_week = now_secs - 7 * 24 * 3600;

    let mut files: Vec<PathBuf> = Vec::new();
    collect_recent_jsonl(&root, cutoff_week, &mut files);

    for path in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if v["type"] != "assistant" {
                continue;
            }
            let ts = v["timestamp"].as_str().map(parse_ts).unwrap_or(0);
            if ts < cutoff_week {
                continue;
            }
            let u = &v["message"]["usage"];
            let inp = u["input_tokens"].as_u64().unwrap_or(0)
                + u["cache_read_input_tokens"].as_u64().unwrap_or(0)
                + u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
            let out = u["output_tokens"].as_u64().unwrap_or(0);
            stats.claude_weekly_in += inp;
            stats.claude_weekly_out += out;
            if ts >= cutoff_5h {
                stats.claude_5h_in += inp;
                stats.claude_5h_out += out;
            }
        }
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
    fn write_rollout(home: &Path, lines: &str) -> PathBuf {
        let now = Local::now();
        let dir = home
            .join(".codex/sessions")
            .join(now.year().to_string())
            .join(format!("{:02}", now.month()))
            .join(format!("{:02}", now.day()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        std::fs::write(&path, lines).unwrap();
        path
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
}
