//! Reconciles the reports DB with GitHub: one issue per problem, body edited in place as counts
//! grow, a comment only when a version is affected for the first time, and a reopen only when a
//! closed issue recurs on a NEWER version than the one it was closed in. Runs in a background
//! worker after the 202; any failure leaves the problem dirty with a backoff, so nothing is lost.

use crate::github::{version_cmp, version_newer};
use crate::reports::{unix_now, ReportsState};
use crate::reports_db::{Problem, VersionCount};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn civil(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        secs / 3600,
        secs % 3600 / 60
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn render_body(p: &Problem, versions: &[VersionCount]) -> String {
    let mut distinct: Vec<&str> = Vec::new();
    for v in versions {
        if !distinct.contains(&v.version.as_str()) {
            distinct.push(&v.version);
        }
    }
    let mut out = format!("<!-- forge-auto-report:{} -->\n", p.fingerprint);
    if p.manual {
        out.push_str("Submitted by a user from Forge's \"Report a problem\".\n\n");
    } else {
        out.push_str("Reported automatically by Forge. All fields are scrubbed; see docs/features/issue-reporting.md.\n\n");
    }
    out.push_str("| | |\n|---|---|\n");
    out.push_str(&format!("| First seen | {} |\n", civil(p.first_seen)));
    out.push_str(&format!("| Last seen | {} |\n", civil(p.last_seen)));
    out.push_str(&format!("| Occurrences | {} |\n", p.count));
    out.push_str(&format!(
        "| Versions affected | {} |\n",
        distinct.join(", ")
    ));
    out.push_str(&format!("| Fingerprint | `{}` |\n\n", p.fingerprint));
    out.push_str("### Occurrences by version and OS\n\n| Version | OS | Count |\n|---|---|---|\n");
    for v in versions {
        out.push_str(&format!("| {} | {} | {} |\n", v.version, v.os, v.count));
    }
    out.push_str("\n### Representative report\n\n```json\n");
    out.push_str(&p.representative.replace("```", "'''"));
    out.push_str("\n```\n");
    out
}

impl ReportsState {
    pub(crate) fn spawn_worker(self: &Arc<Self>) {
        if self.db.is_none() {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = state.wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
                if let Err(error) = state.sync_dirty().await {
                    tracing::warn!("report sync failed: {error}");
                }
            }
        });
    }

    /// Reconcile every problem that is due. Returns how many were attempted.
    pub(crate) async fn sync_dirty(&self) -> anyhow::Result<usize> {
        let (Some(db), Some(_)) = (self.db.as_ref(), self.github.as_ref()) else {
            return Ok(0);
        };
        let _guard = self.sync_lock.lock().await;
        let now = unix_now();
        let due = db.due(now)?;
        for key in &due {
            if let Err(error) = self.sync_problem(key).await {
                tracing::warn!("syncing report {key} failed: {error}");
                db.mark_failed(key, now)?;
            }
        }
        Ok(due.len())
    }

    async fn sync_problem(&self, key: &str) -> anyhow::Result<()> {
        let (Some(db), Some(gh)) = (self.db.as_ref(), self.github.as_ref()) else {
            return Ok(());
        };
        let Some(p) = db.problem(key)? else {
            return Ok(());
        };
        let versions = db.versions(key)?;
        let body = render_body(&p, &versions);
        let hash = hex(&Sha256::digest(body.as_bytes()));
        let labels: &[&str] = if p.manual {
            &["user-report"]
        } else {
            &["auto-report"]
        };
        let mut uncommented: Vec<&str> = Vec::new();
        for v in versions.iter().filter(|v| !v.commented) {
            if !uncommented.contains(&v.version.as_str()) {
                uncommented.push(&v.version);
            }
        }
        let existing = match p.issue_number {
            Some(n) => gh.get_issue(n).await?.map(|state| (n, state)),
            None => None,
        };
        match existing {
            None => {
                let number = gh.create_issue(&p.title, &body, labels).await?;
                db.set_issue_number(key, number)?;
                for v in &versions {
                    db.mark_commented(key, &v.version)?;
                }
                db.set_closed_in(key, None)?;
            }
            Some((number, state)) => {
                let mut reopen = false;
                if state.open {
                    db.set_closed_in(key, None)?;
                } else {
                    let closed_in = p
                        .closed_in_version
                        .clone()
                        .or_else(|| p.synced_max_version.clone())
                        .unwrap_or_else(|| p.max_version.clone());
                    reopen = version_newer(&p.max_version, &closed_in);
                    db.set_closed_in(key, if reopen { None } else { Some(&closed_in) })?;
                    if reopen {
                        gh.update_issue(number, &body, true).await?;
                        gh.comment(
                            number,
                            &format!(
                                "Reopened: this problem recurred on {} (it was closed in {closed_in}).",
                                p.max_version
                            ),
                        )
                        .await?;
                    }
                }
                if !reopen && db.last_body_hash(key).as_deref() != Some(hash.as_str()) {
                    gh.update_issue(number, &body, false).await?;
                }
                for version in &uncommented {
                    if state.open && !reopen {
                        let oses: Vec<&str> = versions
                            .iter()
                            .filter(|v| v.version == *version)
                            .map(|v| v.os.as_str())
                            .collect();
                        gh.comment(
                            number,
                            &format!("First seen on version {version} ({}).", oses.join(", ")),
                        )
                        .await?;
                    }
                    db.mark_commented(key, version)?;
                }
            }
        }
        let newest = versions
            .iter()
            .map(|v| v.version.as_str())
            .max_by(|a, b| version_cmp(a, b))
            .unwrap_or(&p.max_version);
        let next = unix_now() + self.cfg.min_update_secs;
        db.mark_synced(key, newest, &hash, next, "open")?;
        if db.still_dirty_since(key, p.count) {
            db.redirty(key);
        }
        Ok(())
    }
}
