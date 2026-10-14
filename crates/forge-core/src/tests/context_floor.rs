//! The floor's contract: while it stands still, the provider view only ever grows at the end.

use super::*;
use crate::context_pipeline::{to_llm_floored, PRUNE_MARKER, PRUNE_TOOL_RESULT_MAX};
use forge_types::ToolCall;

/// Deterministic xorshift, so a failing case reproduces from its seed without a proptest dep.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// One self-contained piece of conversation. A tool round always carries all of its results, so
/// every generated transcript is one a request could actually be built from.
fn push_unit(rng: &mut Rng, out: &mut Vec<Message>, turn: &mut usize) {
    const GUIDANCE: [&str; 3] = [
        "[lsp diagnostics] warning: unused import",
        "You stopped after announcing your next step.",
        "standing workflow guidance",
    ];
    match rng.below(9) {
        0 => {
            *turn += 1;
            out.push(Message::user(format!(
                "request number {} {}",
                *turn,
                "detail ".repeat(8)
            )));
        }
        1 => out.push(Message::system(GUIDANCE[rng.below(3) as usize])),
        2 => out.push(Message::system(format!(
            "Turn contract: contract variant {}",
            rng.below(3)
        ))),
        3 => out.push(Message::assistant(if rng.below(2) == 0 {
            "I will now look at the parser module and then fix the bug.".to_string()
        } else {
            format!("progress note {}", rng.next())
        })),
        4 => out.push(Message::system("note for the human only").ui_only()),
        _ => {
            let calls = 1 + rng.below(3) as usize;
            let ids: Vec<String> = (0..calls).map(|i| format!("shell:{i}")).collect();
            out.push(Message::assistant_tool_calls(
                "",
                ids.iter()
                    .map(|id| ToolCall {
                        id: id.clone(),
                        name: "shell".into(),
                        args: serde_json::json!({ "command": "cat file" }),
                    })
                    .collect(),
            ));
            for (i, id) in ids.iter().enumerate() {
                let body = match rng.below(4) {
                    0 => "small output".to_string(),
                    1 => "identical big file\n".repeat(80),
                    _ => format!(
                        "result {} {}",
                        rng.next(),
                        "x".repeat(500 + rng.below(4_000) as usize)
                    ),
                };
                out.push(Message::tool_result(id, body));
                if i == 0 && rng.below(4) == 0 {
                    out.push(Message::system(GUIDANCE[0]));
                }
            }
        }
    }
}

fn transcript(rng: &mut Rng, units: usize, turn: &mut usize) -> Vec<Message> {
    let mut out = vec![Message::system("preamble"), Message::user("first request")];
    for _ in 0..units {
        push_unit(rng, &mut out, turn);
    }
    out
}

fn same(a: &Message, b: &Message) -> bool {
    a.role == b.role
        && a.content == b.content
        && a.tool_call_id == b.tool_call_id
        && a.tool_calls.len() == b.tool_calls.len()
        && a.tool_calls
            .iter()
            .zip(&b.tool_calls)
            .all(|(x, y)| x.id == y.id && x.name == y.name && x.args == y.args)
}

fn assert_prefix(before: &[Message], after: &[Message], seed: u64) {
    assert!(after.len() >= before.len(), "seed {seed}: the view shrank");
    for (index, (a, b)) in before.iter().zip(after).enumerate() {
        assert!(
            same(a, b),
            "seed {seed}: message {index} was rewritten after it had been sent:\n{:?}\n--- became ---\n{:?}",
            a.content.chars().take(120).collect::<String>(),
            b.content.chars().take(120).collect::<String>(),
        );
    }
}

/// The property the whole module exists for: with the floor unchanged, `to_llm(T)` is an
/// element-wise prefix of `to_llm(T + E)` — whatever T and E hold, wherever the floor sits.
#[test]
fn an_unchanged_floor_makes_the_view_append_only() {
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let mut turn = 0;
        let units = 4 + rng.below(30) as usize;
        let base = transcript(&mut rng, units, &mut turn);
        let floor = snap_to_boundary(&base, rng.below(base.len() as u64 + 1) as usize);
        let mut grown = base.clone();
        for _ in 0..1 + rng.below(12) {
            push_unit(&mut rng, &mut grown, &mut turn);
        }

        let before = to_llm_floored(&base, 10_000_000, 300, floor, &mut None);
        let after = to_llm_floored(&grown, 10_000_000, 300, floor, &mut None);
        assert_prefix(&before, &after, seed);
    }
}

/// The same holds at the window ceiling while the sticky trim is reused: a session that had to
/// drop old messages keeps appending to the trimmed prompt instead of re-cutting it every call.
#[test]
fn a_reused_trim_keeps_the_view_append_only() {
    let mut checked = 0;
    for seed in 1..=200u64 {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03) | 1);
        let mut turn = 0;
        let units = 10 + rng.below(30) as usize;
        let base = transcript(&mut rng, units, &mut turn);
        let floor = snap_to_boundary(&base, rng.below(base.len() as u64 + 1) as usize);
        let mut grown = base.clone();
        let users_before = grown.iter().filter(|m| m.role == Role::User).count();
        for _ in 0..1 + rng.below(3) {
            push_unit(&mut rng, &mut grown, &mut turn);
        }
        let new_turn = grown.iter().filter(|m| m.role == Role::User).count() > users_before;

        let mut trim = None;
        let before = to_llm_floored(&base, 6_000, 300, floor, &mut trim);
        let mut reused = trim;
        let after = to_llm_floored(&grown, 6_000, 300, floor, &mut reused);
        // A new user message moves the task anchor the trim always keeps, and a view that no longer
        // fits re-trims: both are deliberate breaks, not what this property is about.
        if reused != trim || (trim.is_some() && new_turn) {
            continue;
        }
        checked += 1;
        assert_prefix(&before, &after, seed);
    }
    assert!(checked > 50, "too few comparable cases: {checked}");
}

fn round(id: &str, body: &str) -> [Message; 2] {
    [
        Message::assistant_tool_calls(
            "",
            vec![ToolCall {
                id: id.to_string(),
                name: "read_file".into(),
                args: serde_json::json!({ "path": id }),
            }],
        ),
        Message::tool_result(id, body),
    ]
}

/// Elision and dedupe answer to the same boundary: below the floor a repeat collapses and a bulky
/// result is cut; above it both stay byte-for-byte, however many times they repeat.
#[test]
fn dedupe_and_elision_agree_on_the_floor() {
    let repeated = "the same big file\n".repeat(300);
    let bulky = format!("start {} end", "y".repeat(40_000));
    let mut messages = vec![Message::user("task")];
    messages.extend(round("a", &repeated));
    messages.extend(round("b", &repeated));
    messages.extend(round("c", &bulky));
    let floor = messages.len();
    messages.extend(round("d", &repeated));
    messages.extend(round("e", &bulky));

    let view = to_llm_floored(&messages, 10_000_000, 2_000, floor, &mut None);
    let result = |id: &str| {
        view.iter()
            .find(|m| m.tool_call_id.as_deref() == Some(id))
            .unwrap()
            .content
            .clone()
    };
    assert_eq!(result("a"), repeated, "the first copy is the one kept");
    assert!(
        result("b").contains("identical"),
        "a repeat below the floor collapses"
    );
    assert!(
        result("c").contains("elided"),
        "bulk below the floor is cut"
    );
    assert_eq!(
        result("d"),
        repeated,
        "a repeat above the floor stays verbatim"
    );
    assert_eq!(result("e"), bulky, "bulk above the floor stays verbatim");
}

/// Replaying the advance rule step by step over a growing transcript: the floor never moves
/// backward and never past the working set.
#[test]
fn the_floor_never_moves_backward() {
    let big = "z".repeat(PRUNE_TOOL_RESULT_MAX * 4);
    let mut messages = vec![Message::user("task")];
    let mut floor = 0;
    let mut history = vec![floor];
    for step in 0..60 {
        if step % 15 == 14 {
            messages.push(Message::user(format!("next turn {step}")));
        }
        messages.extend(round(&format!("c{step}"), &big));
        let target = advance_target(&messages, 2, 6);
        if let Some(next) = plan_advance(&messages, floor, target, 8_000, false) {
            assert!(next > floor, "step {step}: {floor} -> {next}");
            floor = next;
        }
        assert!(floor <= messages.len());
        history.push(floor);
    }
    assert!(history.windows(2).all(|w| w[0] <= w[1]), "{history:?}");
    assert!(
        *history.last().unwrap() > 0,
        "the floor did advance: {history:?}"
    );
}

/// The working set — the last rounds and the newest results — is never released, even forced.
#[test]
fn a_forced_advance_still_keeps_the_working_set() {
    let big = "q".repeat(PRUNE_TOOL_RESULT_MAX + 500);
    let mut messages = vec![Message::user("go")];
    for i in 0..(PRUNE_KEEP_ROUNDS + 3) {
        messages.extend(round(&format!("call-{i}"), &big));
    }
    let target = advance_target(&messages, 2, 6);
    let floor = plan_advance(&messages, 0, target, usize::MAX, true).unwrap();
    let kept: std::collections::HashMap<String, String> =
        [("call-0".to_string(), "/tmp/call-0.log".to_string())].into();
    assert!(prune_below(&mut messages, floor, &kept) > 0);
    let cut: Vec<bool> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.content.ends_with(PRUNE_MARKER))
        .collect();
    assert_eq!(cut, [true, true, true, false, false, false, false]);
    assert!(messages[2].content.contains("/tmp/call-0.log"));
    // Idempotent while the floor stands still.
    assert_eq!(prune_below(&mut messages, floor, &kept), 0);
    // Unforced, a block too large to fill does not move it.
    assert_eq!(plan_advance(&messages, 0, target, usize::MAX, false), None);
}

/// Three turns of tool work, replayed through the real advance + prune + view + wire fingerprint
/// path. Every request between floor moves is a pure append; each move costs exactly one break,
/// and moves are at least half a block apart.
#[test]
fn a_replayed_three_turn_session_breaks_only_at_floor_moves() {
    use forge_provider::wire_fingerprint::fingerprint;
    let block = 20_000;
    let kept = std::collections::HashMap::new();
    let mut messages = vec![Message::system("preamble")];
    let mut floor = 0;
    let mut previous = None;
    let mut breaks = 0;
    let mut moves = 0;
    let mut appended_tokens = 0;
    for turn in 0..3 {
        messages.push(Message::user(format!("turn {turn}: do the next part")));
        for step in 0..12 {
            let body = format!("output {turn}-{step} {}", "w".repeat(6_000));
            appended_tokens +=
                crate::context_pipeline::message_tokens(&Message::tool_result("x", &body));
            messages.extend(round(&format!("t{turn}s{step}"), &body));
            let target = advance_target(&messages, 2, 6);
            let moved = match plan_advance(&messages, floor, target, block, false) {
                Some(next) => {
                    floor = next;
                    prune_below(&mut messages, floor, &kept);
                    moves += 1;
                    true
                }
                None => false,
            };
            let view = to_llm_floored(&messages, 10_000_000, 1_000, floor, &mut None);
            let current = fingerprint(&view, &[]);
            if let Some(previous) = &previous {
                let reuse = current.reuse_against(previous);
                if !reuse.is_append() {
                    breaks += 1;
                    assert!(
                        moved,
                        "turn {turn} step {step}: a rewrite without a floor move"
                    );
                }
            }
            previous = Some(current);
        }
    }
    assert!(
        moves > 0,
        "the session grew past several blocks, the floor must move"
    );
    assert!(breaks <= moves, "{breaks} breaks for {moves} floor moves");
    assert!(
        moves <= appended_tokens / (block / 2) + 1,
        "{moves} moves for {appended_tokens} appended tokens: more than one per half block"
    );
}

#[test]
fn a_sticky_trim_is_reused_until_it_no_longer_fits() {
    let mut messages = vec![Message::system("sys")];
    for i in 0..40 {
        messages.push(Message::user(format!(
            "question {i} {}",
            "word ".repeat(60)
        )));
        messages.push(Message::assistant(format!(
            "answer {i} {}",
            "word ".repeat(60)
        )));
    }
    let budget = 3_000;
    let mut trim = None;
    let first = fit_sticky(messages.clone(), budget, &mut trim);
    let dropped = trim.expect("over budget, so a trim was taken");
    let first_cost: usize = first.iter().map(message_tokens).sum();
    assert!(
        first_cost <= budget * 85 / 100 + 400,
        "it cut down towards 85%: {first_cost}"
    );

    messages.push(Message::assistant("a short follow-up"));
    let mut reused = trim;
    let second = fit_sticky(messages.clone(), budget, &mut reused);
    assert_eq!(
        reused,
        Some(dropped),
        "the same cut is reused while it fits"
    );
    assert_eq!(second.len(), first.len() + 1);

    // A smaller window after failover cannot hold the old cut: it re-fits.
    let mut smaller = trim;
    let third = fit_sticky(messages, 1_200, &mut smaller);
    assert!(smaller.unwrap() > dropped);
    assert!(third.iter().map(message_tokens).sum::<usize>() <= 1_200);
}
