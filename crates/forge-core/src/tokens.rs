//! Real BPE token counting for precise context accounting.
//!
//! The old approach multiplied a character count by a fixed chars-per-token guess. That is wildly
//! inaccurate for a coding agent's transcript (code, JSON, tool output tokenize very differently
//! from prose), which made the context gauge lie and let the trimmed input overflow the model's
//! window. Here we count tokens with a real BPE tokenizer (`o200k_base`, the GPT-4o/5 vocabulary
//! bundled in-crate, so it works offline). It is exact for OpenAI/Codex models and a close
//! approximation for others — Claude/Gemini/Llama tokenizers differ only modestly in magnitude —
//! which is far better than a chars/token heuristic for deciding when to trim and compact.
//!
//! Counts are cached by content hash: transcript messages are immutable once recorded, so the
//! (relatively expensive) BPE pass runs once per distinct string and is essentially free thereafter
//! — important because the gauge recomputes the whole transcript frequently.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use tiktoken_rs::{o200k_base, CoreBPE};

fn bpe() -> &'static CoreBPE {
    static BPE: OnceLock<CoreBPE> = OnceLock::new();
    BPE.get_or_init(|| {
        let started = std::time::Instant::now();
        let bpe = o200k_base().expect("bundled o200k_base vocabulary always loads");
        tracing::debug!(
            target: "forge::startup",
            "tokenizer vocabulary built on thread {:?} in {} ms",
            std::thread::current().name().unwrap_or("?"),
            started.elapsed().as_millis()
        );
        bpe
    })
}

/// Build the vocabulary on a background thread. Loading `o200k_base` takes ~150 ms of CPU and about
/// 45 MB, and the first count happens while the first turn routes, so a surface that is about to
/// run a turn calls this at startup to take that load off the first token. A count that arrives
/// while the thread is still building waits for it instead of building a second copy.
///
/// A surface where a person types the first prompt calls [`defer_warm_until_input`] first and
/// [`warm_now`] at the first input event instead: a session that is opened and left idle then never
/// holds the vocabulary, and typing a prompt takes longer than building it.
pub fn warm_in_background() {
    if WARM.background() {
        spawn_warm();
    }
}

/// From here on [`warm_in_background`] does nothing; [`warm_now`] starts the build.
pub fn defer_warm_until_input() {
    WARM.defer();
}

/// Start building the vocabulary on a background thread, once per process. A single atomic load
/// after the first call, so it is cheap enough for an input loop.
pub fn warm_now() {
    if WARM.now() {
        spawn_warm();
    }
}

fn spawn_warm() {
    let _ = std::thread::Builder::new()
        .name("forge-tokenizer-warm".into())
        .spawn(|| {
            bpe();
        });
}

static WARM: WarmGate = WarmGate::new();

/// Decides when the vocabulary build may start; pure over its own flags so the rule is testable
/// without touching the process-wide gate.
struct WarmGate {
    deferred: AtomicBool,
    started: AtomicBool,
}

impl WarmGate {
    const fn new() -> Self {
        Self {
            deferred: AtomicBool::new(false),
            started: AtomicBool::new(false),
        }
    }

    fn defer(&self) {
        self.deferred.store(true, Ordering::Relaxed);
    }

    /// Whether a startup-time warm-up should spawn the build now.
    fn background(&self) -> bool {
        !self.deferred.load(Ordering::Relaxed) && self.now()
    }

    /// Whether this call is the one that should spawn the build (true once).
    fn now(&self) -> bool {
        !self.started.swap(true, Ordering::Relaxed)
    }
}

fn cache() -> &'static Mutex<HashMap<u64, usize>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, usize>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn args_cache() -> &'static Mutex<HashMap<u64, usize>> {
    static CACHE: OnceLock<Mutex<HashMap<u64, usize>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Per-message framing overhead (role marker + delimiters) the model also pays, mirrored so the
/// transcript estimate tracks the real request size. ~4 tokens is the standard chat-format budget.
pub const PER_MESSAGE_OVERHEAD: usize = 4;

/// Exact BPE token count of `text` (no framing). Cached by content hash so repeated counts of the
/// same (immutable) message are O(1).
pub fn count_text(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    let key = h.finish();
    // Recover from a poisoned lock instead of cascading the panic: a panic elsewhere while holding
    // this purely-advisory token cache must not take down every later `count_text` call.
    if let Some(&n) = cache().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return n;
    }
    let n = bpe().encode_with_special_tokens(text).len();
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, n);
    n
}

/// Token count of one message including the chat-format framing overhead.
pub fn count_message(content: &str) -> usize {
    count_text(content) + PER_MESSAGE_OVERHEAD
}

/// Token count of a tool call's JSON arguments, memoized by the call's stable `id`. A tool call's
/// args are immutable once recorded, so the (relatively expensive) `args.to_string()` serialization
/// runs at most once per call rather than on every transcript-estimation pass. Falls back to a
/// direct, uncached count when the call has no id.
pub fn count_tool_args(call_id: &str, args: &serde_json::Value) -> usize {
    if call_id.is_empty() {
        return count_text(&args.to_string());
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    call_id.hash(&mut h);
    let key = h.finish();
    if let Some(&n) = args_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
    {
        return n;
    }
    let n = count_text(&args.to_string());
    args_cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, n);
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_nonzero_and_scale_with_length() {
        assert_eq!(count_text(""), 0);
        let short = count_text("hello world");
        let long = count_text(&"hello world ".repeat(100));
        assert!(short > 0, "non-empty text has tokens");
        assert!(
            long > short * 50,
            "100x the text is ~100x the tokens: {short} vs {long}"
        );
    }

    #[test]
    fn a_deferred_warm_up_waits_for_the_first_input_and_then_runs_once() {
        let gate = WarmGate::new();
        gate.defer();
        assert!(!gate.background(), "the startup warm-up is skipped");
        assert!(!gate.background(), "and stays skipped");
        assert!(gate.now(), "the first input starts the build");
        assert!(!gate.now(), "once");

        let eager = WarmGate::new();
        assert!(eager.background(), "without a deferral startup builds it");
        assert!(!eager.background());
        assert!(!eager.now(), "an input after that finds it started");
    }

    #[test]
    fn caching_returns_the_same_count() {
        let s = "the quick brown fox jumps over the lazy dog";
        assert_eq!(count_text(s), count_text(s));
    }

    #[test]
    fn message_adds_framing_overhead() {
        assert_eq!(count_message("x"), count_text("x") + PER_MESSAGE_OVERHEAD);
    }

    #[test]
    fn tool_args_memo_matches_direct_count_and_is_stable() {
        let args = serde_json::json!({ "path": "src/lib.rs", "start": 1, "end": 200 });
        let direct = count_text(&args.to_string());
        // Cache miss then hit — both agree with the un-memoized count.
        assert_eq!(count_tool_args("call_abc", &args), direct);
        assert_eq!(count_tool_args("call_abc", &args), direct);
        // Empty id falls back to a direct count rather than caching under a shared key.
        assert_eq!(count_tool_args("", &args), direct);
    }
}
