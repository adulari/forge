//! Preview URLs as PR evidence: each expose is recorded in the session's evidence ledger, and
//! `open_pr` asks which of them are still reachable before it lists them.

use std::path::Path;

use super::background::list;
use super::preview::TunnelRecord;

/// Record a freshly opened preview URL against the current session. Best-effort: a ledger write
/// failure must never fail the expose that already put the URL up.
pub(super) fn record_evidence(job: u32, record: &TunnelRecord) {
    if let Some(ledger) = crate::evidence::Ledger::current() {
        let _ = ledger.record_preview(&record.url, job, record.port);
    }
}

/// Preview URLs in `workspace` whose job is running and whose tunnel process is still the one
/// that was started.
pub fn live_preview_urls(workspace: &Path) -> Vec<String> {
    let cwd = workspace.display().to_string();
    list(Some(workspace), &cwd)
        .into_iter()
        .filter(|job| job.is_running())
        .filter_map(|job| job.tunnel)
        .filter(TunnelRecord::is_alive)
        .map(|tunnel| tunnel.url)
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_without_jobs_has_no_live_previews() {
        let dir = tempfile::tempdir().unwrap();
        assert!(live_preview_urls(dir.path()).is_empty());
    }

    #[tokio::test]
    async fn expose_evidence_lands_in_the_scoped_session_ledger() {
        let dir = tempfile::tempdir().unwrap();
        let record = TunnelRecord {
            pid: 1,
            start_ticks: None,
            port: 5173,
            url: "https://x.trycloudflare.com".to_string(),
            provider: "cloudflared".to_string(),
            started_at: 0,
            log: dir.path().join("t.log"),
        };
        crate::evidence::scope_session("prev-1".to_string(), dir.path().to_path_buf(), async {
            record_evidence(77, &record);
        })
        .await;
        let items = crate::evidence::Ledger::new(dir.path(), "prev-1")
            .unwrap()
            .items();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, crate::evidence::EvidenceKind::Preview);
        assert_eq!(items[0].url.as_deref(), Some("https://x.trycloudflare.com"));
        assert_eq!(items[0].caption.as_deref(), Some("job 77 port 5173"));
    }
}
