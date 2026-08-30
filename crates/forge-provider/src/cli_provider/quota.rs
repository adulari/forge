use serde_json::Value;

/// Normalise a Claude `rateLimitType` to Forge's window vocabulary (`five_hour` / `weekly`).
/// Claude uses `seven_day` for the weekly window; everything else passes through unchanged.
pub(super) fn normalize_window(rate_limit_type: &str) -> String {
    let t = rate_limit_type.to_lowercase();
    if t.contains("seven") || t.contains("week") || t == "7d" {
        "weekly".to_string()
    } else if t.contains("five") || t.contains("5h") || t == "hour" {
        "five_hour".to_string()
    } else {
        rate_limit_type.to_string()
    }
}

/// Map a Claude `rate_limit_info` into a coarse [`QuotaStatus`], defensively (the schema is
/// version-volatile). We read the live `status` + `isUsingOverage` (NOT `overageStatus`, which is
/// a setting, not the current state) plus a usage fraction when present. Unknown → `Ok`.
pub(super) fn quota_status_from(
    status: &str,
    using_overage: bool,
    fraction: Option<f64>,
) -> forge_types::QuotaStatus {
    use forge_types::QuotaStatus;
    let s = status.to_lowercase();
    if s.contains("reject") || s.contains("block") || s.contains("exceed") || s.contains("exhaust")
    {
        return QuotaStatus::Exhausted;
    }
    if let Some(f) = fraction {
        let by_fraction = forge_config::quota_status::status_from_fraction(f);
        if by_fraction != QuotaStatus::Ok {
            return by_fraction;
        }
    }
    if using_overage || s.contains("warn") || s.contains("approach") {
        return QuotaStatus::Warning;
    }
    QuotaStatus::Ok
}

/// Whether a Codex rollout `rate_limits` object describes the account-wide ChatGPT Codex quota.
///
/// Recent Codex builds also emit model-specific limits such as `codex_bengalfox` for
/// GPT-5.3-Codex-Spark. Those values are real, but they are not interchangeable with the shared
/// Codex allowance used to route `codex-cli` and `codex-oauth`, so they must never update that
/// bucket. Older rollout records omitted `limit_id`; retain them as a backward-compatible
/// account-wide observation.
pub fn codex_rollout_is_account_wide(rate_limits: &Value) -> bool {
    rate_limits
        .get("limit_id")
        .and_then(Value::as_str)
        .is_none_or(|id| id.trim().eq_ignore_ascii_case("codex"))
}

/// Build [`QuotaHint`]s for ALL non-stale windows from a Codex session rollout JSONL.
/// Returns one entry per window (primary = 5h, secondary = weekly) that is still active.
pub(super) fn codex_quota_from_rollout(jsonl: &str, provider: &str) -> Vec<forge_types::QuotaHint> {
    let rl = jsonl.lines().rev().find_map(|line| {
        let v: Value = serde_json::from_str(line.trim()).ok()?;
        let p = v.get("payload").unwrap_or(&v);
        if p.get("type").and_then(Value::as_str) != Some("token_count") {
            return None;
        }
        let rate_limits = p.get("rate_limits").filter(|r| r.is_object())?;
        codex_rollout_is_account_wide(rate_limits).then(|| rate_limits.clone())
    });
    let Some(rl) = rl else {
        return Vec::new();
    };

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let reached_type = rl.get("rate_limit_reached_type").and_then(Value::as_str);

    let mut hints = Vec::new();
    for (key, reached_key) in [("primary", "primary"), ("secondary", "secondary")] {
        let Some(w) = rl.get(key) else { continue };
        let Some(used) = w.get("used_percent").and_then(Value::as_f64) else {
            continue;
        };
        let resets = w.get("resets_at").and_then(Value::as_i64);
        let mins = w.get("window_minutes").and_then(Value::as_i64).unwrap_or(0);
        // Skip stale windows (the period has already reset).
        if let Some(r) = resets {
            if r <= now_secs {
                continue;
            }
        }
        let fraction = used / 100.0;
        let reached = reached_type.is_some_and(|rt| rt == reached_key);
        let status = if reached {
            forge_types::QuotaStatus::Exhausted
        } else {
            quota_status_from("", false, Some(fraction))
        };
        let label = match mins {
            300 => "five_hour".to_string(),
            10080 => "weekly".to_string(),
            m if m > 0 => format!("{m}m"),
            _ => key.to_string(),
        };
        hints.push(forge_types::QuotaHint {
            provider: provider.to_string(),
            window: label,
            status,
            resets_at: resets,
            fraction_used: Some(fraction),
        });
    }
    hints
}

/// `${CODEX_HOME:-~/.codex}/sessions`, where codex writes its rollout files.
fn codex_sessions_dir() -> Option<std::path::PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return Some(std::path::PathBuf::from(h).join("sessions"));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        std::path::PathBuf::from(home)
            .join(".codex")
            .join("sessions"),
    )
}

/// `${CODEX_HOME:-~/.codex}/auth.json`, where the official `codex` CLI stores its own OAuth
/// tokens (same directory scheme as [`codex_sessions_dir`]).
fn codex_auth_json_path() -> Option<std::path::PathBuf> {
    if let Some(h) = std::env::var_os("CODEX_HOME") {
        return Some(std::path::PathBuf::from(h).join("auth.json"));
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        std::path::PathBuf::from(home)
            .join(".codex")
            .join("auth.json"),
    )
}

/// The `codex` CLI's own ChatGPT plan: `tokens.access_token`'s `chatgpt_plan_type` claim from
/// `~/.codex/auth.json` (see docs/design/subscription-efficiency-routing.md Fix 4). Since
/// `codex-cli` and `codex-oauth` share the same ChatGPT account, the two surfaces cannot
/// disagree by construction. `None` when the file is missing, unreadable, malformed, or the
/// token carries no plan claim — tolerant by design, never logs token material.
pub fn codex_cli_detected_plan() -> Option<String> {
    let path = codex_auth_json_path()?;
    let body = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&body).ok()?;
    let access_token = v.get("tokens")?.get("access_token")?.as_str()?;
    forge_config::provider_oauth::extract_chatgpt_plan_type(access_token)
}

/// Find the rollout file for a codex thread (`rollout-<ts>-<thread_id>.jsonl`) under the sessions
/// dir (organised `YYYY/MM/DD/`). Bounded recursion; returns the first match.
pub(super) fn find_codex_rollout(thread_id: &str) -> Option<std::path::PathBuf> {
    fn search(dir: &std::path::Path, suffix: &str, depth: u8) -> Option<std::path::PathBuf> {
        if depth == 0 {
            return None;
        }
        let mut subdirs = Vec::new();
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_dir() {
                subdirs.push(path);
            } else if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(suffix))
            {
                return Some(path);
            }
        }
        subdirs
            .into_iter()
            .find_map(|d| search(&d, suffix, depth - 1))
    }
    let dir = codex_sessions_dir()?;
    search(&dir, &format!("-{thread_id}.jsonl"), 5)
}
