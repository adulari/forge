use super::*;

fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().join("reports"));
    (dir, store)
}

fn bug(site: &str, msg: &str) -> Incident {
    Incident::bug(ErrorKind::Internal, site, msg)
}

#[test]
fn off_collects_nothing() {
    let (_dir, store) = store();
    let d = report_in(
        bug("t::off", "boom"),
        ReportingMode::Off,
        Some(&store),
        true,
        &Guards::new(),
    );
    assert_eq!(d, Disposition::Off);
    assert!(store.queued().is_empty());
    assert!(store.last().is_none());
}

#[test]
fn builds_that_may_not_send_collect_nothing() {
    let (_dir, store) = store();
    let d = report_in(
        bug("t::nosend", "boom"),
        ReportingMode::Auto,
        Some(&store),
        false,
        &Guards::new(),
    );
    assert_eq!(d, Disposition::Off);
    assert!(store.queued().is_empty());
}

#[test]
fn expected_conditions_are_never_queued() {
    let (_dir, store) = store();
    let d = report_in(
        Incident::gray(ErrorKind::ProviderProtocol, "t::rl", "429 rate limited"),
        ReportingMode::Auto,
        Some(&store),
        true,
        &Guards::new(),
    );
    assert_eq!(d, Disposition::NotABug(Expected::RateLimit));
    assert!(store.queued().is_empty());
}

#[test]
fn ask_mode_holds_and_resolution_moves_it_to_the_queue() {
    let (_dir, store) = store();
    let d = report_in(
        bug("t::ask", "boom ask"),
        ReportingMode::Ask,
        Some(&store),
        true,
        &Guards::new(),
    );
    assert_eq!(d, Disposition::Held);
    assert!(store.queued().is_empty());
    let pending = store.pending();
    assert_eq!(pending.len(), 1);
    let report = store.take_pending(&pending[0].0).unwrap();
    assert_eq!(report.site, "t::ask");
    assert!(store.pending().is_empty());
}

#[test]
fn auto_mode_queues_and_dedups_the_same_problem() {
    let (_dir, store) = store();
    let guards = Guards::new();
    let first = report_in(
        bug("t::dedup", "boom 1"),
        ReportingMode::Auto,
        Some(&store),
        true,
        &guards,
    );
    assert_eq!(first, Disposition::Queued);
    let second = report_in(
        bug("t::dedup", "boom 2"),
        ReportingMode::Auto,
        Some(&store),
        true,
        &guards,
    );
    assert_eq!(second, Disposition::Duplicate);
    assert_eq!(store.queued().len(), 1);
}

#[test]
fn the_report_is_closed_scrubbed_and_content_free() {
    let (_dir, store) = store();
    let secret = "sk-live-abcdefghijklmnopqrstuvwxyz012345";
    let incident = Incident::bug(
        ErrorKind::Internal,
        "t::privacy",
        format!(
            "failed on \"my confidential prompt\" at /home/bob/private/notes.md with {secret} via https://corp.internal/path"
        ),
    );
    report_in(
        incident,
        ReportingMode::Auto,
        Some(&store),
        true,
        &Guards::new(),
    );
    let json = std::fs::read_to_string(store.dir().join("last.json")).unwrap();
    for leaked in [
        "confidential prompt",
        "/home/bob/private",
        secret,
        "corp.internal/path",
    ] {
        assert!(!json.contains(leaked), "leaked {leaked}");
    }
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for key in keys {
        assert!(
            [
                "v",
                "source",
                "install_id",
                "fingerprint",
                "error_kind",
                "site",
                "location",
                "message",
                "frames",
                "context",
                "events",
                "occurred_at"
            ]
            .contains(&key),
            "unexpected field {key}"
        );
    }
}

#[test]
fn install_id_is_random_stable_hex() {
    let (_dir, store) = store();
    let a = store.install_id();
    assert_eq!(a.len(), 32);
    assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(a, store.install_id());
}

#[test]
fn queue_is_bounded_and_sent_log_keeps_the_latest() {
    let (_dir, store) = store();
    for i in 0..(store::MAX_QUEUE + 7) {
        let mut r = build_report(&bug("t::bound", "x"));
        r.occurred_at = 1_000 + i as u64;
        r.fingerprint = format!("{i:016x}");
        store.enqueue(&r).unwrap();
    }
    assert_eq!(store.queued().len(), store::MAX_QUEUE);
    for _ in 0..(store::MAX_SENT_LOG + 5) {
        store.append_sent(&build_report(&bug("t::log", "x")));
    }
    assert_eq!(store.sent().len(), store::MAX_SENT_LOG);
}

#[test]
fn manual_reports_carry_the_users_words_scrubbed_and_no_error_unless_given() {
    let report = build_manual(
        "it crashed in /home/carol/app when I pasted sk-test-abcdefghijklmnop12345",
        None,
    );
    assert_eq!(report.source, Source::Manual);
    assert_eq!(report.fingerprint, MANUAL_FINGERPRINT);
    let note = report.user_note.unwrap();
    assert!(!note.contains("sk-test"), "{note}");
    assert!(report.last_error.is_none());
    let with_error = build_manual("x", Some("boom at /home/carol/f.rs:1"));
    assert!(!with_error.last_error.unwrap().contains("/home/carol"));
}

#[test]
fn backoff_marker_round_trips() {
    let (_dir, store) = store();
    assert!(!store.in_backoff());
    store.set_backoff_until(store::now_secs() + 600);
    assert!(store.in_backoff());
}

mod relay {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    /// One-shot HTTP server answering every request with `status` (and `retry-after` when set).
    fn serve(
        status: u16,
        retry_after: Option<u64>,
    ) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/reports", listener.local_addr().unwrap());
        listener.set_nonblocking(false).unwrap();
        let handle = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while std::time::Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_millis(500)))
                    .unwrap();
                let mut buf = vec![0u8; 65536];
                let mut total = 0;
                loop {
                    match stream.read(&mut buf[total..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            total += n;
                            let text = String::from_utf8_lossy(&buf[..total]).to_string();
                            if let Some(i) = text.find("\r\n\r\n") {
                                let len = text
                                    .lines()
                                    .find_map(|l| {
                                        l.to_ascii_lowercase()
                                            .strip_prefix("content-length: ")
                                            .and_then(|v| v.trim().parse::<usize>().ok())
                                    })
                                    .unwrap_or(0);
                                if total >= i + 4 + len {
                                    break;
                                }
                            }
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf[..total]).to_string();
                bodies.push(
                    text.split("\r\n\r\n")
                        .nth(1)
                        .unwrap_or_default()
                        .to_string(),
                );
                let retry = retry_after
                    .map(|s| format!("retry-after: {s}\r\n"))
                    .unwrap_or_default();
                let _ = write!(stream, "HTTP/1.1 {status} X\r\n{retry}content-length: 2\r\nconnection: close\r\n\r\n{{}}");
                if bodies.len() >= 3 {
                    break;
                }
            }
            bodies
        });
        (url, handle)
    }

    fn queue_one(store: &Store) {
        store.enqueue(&build_report(&bug("t::send", "x"))).unwrap();
    }

    #[test]
    fn a_successful_send_clears_the_queue_and_logs_it() {
        let (_dir, store) = store();
        queue_one(&store);
        let (url, server) = serve(202, None);
        let stats = send::flush(&store, &url, Duration::from_secs(3));
        assert_eq!(stats.sent, 1);
        assert!(store.queued().is_empty());
        assert_eq!(store.sent().len(), 1);
        let bodies = server.join().unwrap();
        assert!(bodies[0].contains("\"fingerprint\""));
    }

    #[test]
    fn concurrent_flushers_never_send_the_same_report_twice() {
        let (_dir, store) = store();
        queue_one(&store);
        let (url, server) = serve(202, None);
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (s, u) = (store.clone(), url.clone());
                std::thread::spawn(move || send::flush(&s, &u, Duration::from_secs(3)).sent)
            })
            .collect();
        let sent: usize = handles.into_iter().map(|h| h.join().unwrap()).sum();
        assert_eq!(sent, 1);
        assert_eq!(server.join().unwrap().len(), 1);
    }

    #[test]
    fn a_429_pauses_sending_and_keeps_the_report() {
        let (_dir, store) = store();
        queue_one(&store);
        let (url, server) = serve(429, Some(120));
        let stats = send::flush(&store, &url, Duration::from_secs(3));
        assert!(stats.paused);
        assert_eq!(store.queued().len(), 1);
        assert!(store.in_backoff());
        let again = send::flush(&store, &url, Duration::from_secs(1));
        assert!(again.paused);
        drop(server);
    }

    #[test]
    fn a_503_without_retry_after_still_backs_off() {
        let (_dir, store) = store();
        queue_one(&store);
        let (url, server) = serve(503, None);
        let stats = send::flush(&store, &url, Duration::from_secs(3));
        assert!(stats.paused && store.in_backoff());
        drop(server);
    }

    #[test]
    fn a_400_drops_the_report() {
        let (_dir, store) = store();
        queue_one(&store);
        let (url, server) = serve(400, None);
        let stats = send::flush(&store, &url, Duration::from_secs(3));
        assert_eq!(stats.dropped, 1);
        assert!(store.queued().is_empty());
        assert!(store.sent().is_empty());
        drop(server);
    }

    #[test]
    fn an_unreachable_relay_keeps_the_report_for_later() {
        let (_dir, store) = store();
        queue_one(&store);
        let stats = send::flush(
            &store,
            "http://127.0.0.1:9/v1/reports",
            Duration::from_secs(3),
        );
        assert!(stats.paused);
        assert_eq!(store.queued().len(), 1);
    }
}
