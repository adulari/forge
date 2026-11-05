//! cargo run -p forge_mobile_core --example smoke -- <base_url> [session_id]
use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};
use forge_mobile_core::*;

struct L(AtomicUsize);
impl SessionListener for L {
    fn on_update(&self, u: SessionUpdate) {
        let n = self.0.fetch_add(1, Ordering::Relaxed);
        if n % 25 == 0 { println!("update rev={} busy={} chars={} blocks={}", u.revision, u.busy, u.streaming_chars, u.streaming_blocks.len()); }
    }
    fn on_items(&self, items: Vec<ChatItem>) { println!("items: {:?}", items.iter().map(|i| (i.seq, i.role)).collect::<Vec<_>>()); }
    fn on_connection(&self, s: ConnectionState) { println!("conn {s:?}"); }
}

#[tokio::main]
async fn main() {
    let a: Vec<String> = std::env::args().collect();
    let c = DaemonClient::new(a[1].clone());
    let fleet = c.fleet().await.unwrap();
    println!("fleet entries: {}", fleet.len());
    for e in fleet.iter().take(6) { match e { FleetEntry::Label{text} => println!("  [{text}]"), FleetEntry::Session{row} => println!("  {} waiting={} busy={}", row.title, row.waiting, row.busy) } }
    let sid = a.get(2).cloned().unwrap_or_else(|| match fleet.iter().find_map(|e| if let FleetEntry::Session{row} = e { Some(row.id.clone()) } else { None }) { Some(i) => i, None => panic!("no sessions") });
    let s = c.session(sid);
    let t = std::time::Instant::now();
    let h = s.load_history(1000).await.unwrap();
    println!("history {} rows in {:?}", h.len(), t.elapsed());
    s.start_stream(Arc::new(L(AtomicUsize::new(0))));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    s.send_prompt("hello from rust **core**".into()).unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(9)).await;
    s.stop_stream();
}
