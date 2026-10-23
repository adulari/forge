//! Whether an interactive `forge chat` still owns a session, for the daemon's decision to take one
//! over (restore it after a restart, archive it, delete it).
//!
//! The store's presence heartbeat (`LOCAL_PRESENCE_STALE_SECS`) is the cheap answer, but it only
//! proves the terminal's main loop ran in the last minute. A chat whose loop is blocked — a
//! stalled disk during a deploy, a process just resumed from a long stop — has stopped beating
//! while the process, its in-memory transcript and its control channel are all still there.
//! Treating that as "gone" made the restarted daemon resume the session beside it, two drivers on
//! one transcript, and the resume cleared the chat's presence so it never reappeared. The control
//! channel is served from the chat's runtime, not its main loop, so an answer from it settles the
//! question.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use forge_store::Store;

/// A claim silent for longer than this is an abandoned row, not a blocked terminal: a process
/// that has not run its loop in half an hour is not one anybody is waiting on, and the loopback
/// port it recorded has long since been free for something else to take.
const MAX_BLOCKED_SECS: i64 = 30 * 60;

const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// Whether a live terminal process owns `session_id`: a fresh heartbeat, or a quiet one whose
/// published control channel still answers.
pub(crate) async fn terminal_owns_session(store: &Arc<Store>, session_id: &str) -> bool {
    let lookup = {
        let store = Arc::clone(store);
        let id = session_id.to_string();
        tokio::task::spawn_blocking(move || store.local_claim(&id)).await
    };
    let Some(claim) = lookup.ok().and_then(Result::ok).flatten() else {
        return false;
    };
    if claim.is_fresh() {
        return true;
    }
    if claim.idle_secs > MAX_BLOCKED_SECS {
        return false;
    }
    match claim.control_url.as_deref() {
        Some(url) => control_channel_answers(url).await,
        None => false,
    }
}

/// The loopback address of a published control URL. Anything else is refused: the value comes
/// from the database, and the daemon must not be steered into requesting an arbitrary host.
fn loopback_addr(url: &str) -> Option<SocketAddr> {
    let parsed = reqwest::Url::parse(url).ok()?;
    let port = parsed.port_or_known_default()?;
    let host = parsed.host_str()?;
    let ip: IpAddr = if host == "localhost" {
        IpAddr::from([127, 0, 0, 1])
    } else {
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .parse()
            .ok()?
    };
    ip.is_loopback().then(|| SocketAddr::new(ip, port))
}

/// Whether `url` is answered by a Forge control server: its token path serves the control page,
/// which nothing else listening on a recycled port will do.
async fn control_channel_answers(url: &str) -> bool {
    let Some(addr) = loopback_addr(url) else {
        return false;
    };
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let target = format!("http://{addr}{}", parsed.path());
    let Ok(client) = reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() else {
        return false;
    };
    client
        .get(target)
        .send()
        .await
        .is_ok_and(|response| response.status().is_success())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store on disk (not `:memory:`), so a second connection can age a heartbeat.
    fn file_store() -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path().join("owner.db")).unwrap());
        (dir, store)
    }

    fn silent_for(dir: &tempfile::TempDir, id: &str, secs: i64) {
        rusqlite::Connection::open(dir.path().join("owner.db"))
            .unwrap()
            .execute(
                "UPDATE session SET local_last_seen = strftime('%s','now') - ?2 WHERE id = ?1",
                rusqlite::params![id, secs],
            )
            .unwrap();
    }

    fn claim(store: &Store, id: &str, control_url: Option<&str>) {
        store.set_session_local_live(id, true).unwrap();
        if let Some(url) = control_url {
            store.set_session_local_control_url(id, Some(url)).unwrap();
        }
    }

    async fn control_server(token_path: &'static str) -> SocketAddr {
        let app =
            axum::Router::new().route(token_path, axum::routing::get(|| async { "control page" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn a_session_no_terminal_holds_is_not_owned() {
        let (_dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        assert!(!terminal_owns_session(&store, &id).await);
    }

    #[tokio::test]
    async fn a_fresh_heartbeat_is_ownership_without_any_probe() {
        let (_dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        claim(&store, &id, None);
        assert!(terminal_owns_session(&store, &id).await);
    }

    #[tokio::test]
    async fn a_blocked_terminal_is_still_the_owner_while_its_control_channel_answers() {
        let addr = control_server("/tok").await;
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        claim(&store, &id, Some(&format!("http://{addr}/tok")));
        silent_for(&dir, &id, 5 * 60);
        assert!(
            terminal_owns_session(&store, &id).await,
            "five silent minutes with a live control channel is a stalled chat, not a dead one"
        );
    }

    #[tokio::test]
    async fn a_killed_terminal_is_not_the_owner() {
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        // Port 1 on loopback refuses connections: nothing is listening where the chat was.
        claim(&store, &id, Some("http://127.0.0.1:1/tok"));
        silent_for(&dir, &id, 5 * 60);
        assert!(!terminal_owns_session(&store, &id).await);
    }

    #[tokio::test]
    async fn a_quiet_terminal_that_never_published_a_channel_is_not_the_owner() {
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        claim(&store, &id, None);
        silent_for(&dir, &id, 5 * 60);
        assert!(!terminal_owns_session(&store, &id).await);
    }

    #[tokio::test]
    async fn a_recycled_port_that_is_not_a_control_server_does_not_count() {
        let addr = control_server("/some-other-path").await;
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        claim(&store, &id, Some(&format!("http://{addr}/tok")));
        silent_for(&dir, &id, 5 * 60);
        assert!(!terminal_owns_session(&store, &id).await);
    }

    #[tokio::test]
    async fn a_claim_silent_for_hours_is_abandoned_even_if_the_port_answers() {
        let addr = control_server("/tok").await;
        let (dir, store) = file_store();
        let id = store.create_session("/x", "default").unwrap();
        claim(&store, &id, Some(&format!("http://{addr}/tok")));
        silent_for(&dir, &id, 3 * 3600);
        assert!(!terminal_owns_session(&store, &id).await);
    }

    #[test]
    fn only_loopback_control_urls_are_ever_probed() {
        assert_eq!(
            loopback_addr("http://127.0.0.1:4123/tok"),
            Some("127.0.0.1:4123".parse().unwrap())
        );
        assert_eq!(
            loopback_addr("http://localhost:4123/tok"),
            Some("127.0.0.1:4123".parse().unwrap())
        );
        assert_eq!(
            loopback_addr("http://[::1]:4123/tok"),
            Some("[::1]:4123".parse().unwrap())
        );
        assert_eq!(loopback_addr("http://192.168.1.9:4123/tok"), None);
        assert_eq!(loopback_addr("http://example.com/tok"), None);
        assert_eq!(loopback_addr("not a url"), None);
    }
}
