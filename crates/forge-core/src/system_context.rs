//! The provider-facing view of system-role messages in a transcript.

use forge_types::{Message, Role};

pub(crate) const TURN_CONTRACT_PREFIX: &str = "Turn contract:";

/// Derive the provider's system-context view without rewriting the persisted transcript.
///
/// Every decision depends only on the message itself and the ones BEFORE it, so a transcript that
/// grows never changes what the provider was already sent — prompt caches key on a prefix, and a
/// rule that looks ahead rewrites the middle of the prompt the moment a later message arrives.
///
/// Exact repeated system guidance is standing context, not cumulative evidence, so only one full
/// copy is sent, and it is the FIRST copy: keeping the newest deleted the previously-kept one from
/// the middle of the prompt. One live session showed 344 exact-duplicate system messages, mostly
/// re-emitted `[lsp diagnostics]` blocks: 344 invalidations of a ~50k-token prompt, each to save
/// ~380 chars.
///
/// A repeat that directly follows an assistant message is kept: it is the harness reacting to what
/// the model just said (the end-of-turn review, the "prove it" completion gate, a stalled-step
/// nudge). Its text is identical every turn, so deduplicating it deleted the reaction from the
/// request entirely — the model was re-driven with nothing new to act on, and a persistent CLI
/// bridge answered "nothing new is requested" as the turn's final reply. (This used to also check
/// that the NEXT message was not a user prompt, which made the view depend on later messages.)
///
/// Turn contracts stay where they were injected — a newer one says it supersedes the earlier ones
/// (`turn_contract::contract_message`). Only below `floor` (an index into `messages`, see
/// `context_floor`) is a contract that a later contract, also below the floor, superseded dropped:
/// that region is only rewritten when the floor advances, which is a deliberate cache break anyway.
///
/// Returns the view and how many of its messages came from `messages[..floor]`.
pub(crate) fn normalize_system_context(
    messages: Vec<Message>,
    floor: usize,
) -> (Vec<Message>, usize) {
    let is_contract = |message: &Message| {
        message.role == Role::System
            && message
                .content
                .trim_start()
                .starts_with(TURN_CONTRACT_PREFIX)
    };
    let newest_contract_below_floor = messages[..floor.min(messages.len())]
        .iter()
        .rposition(is_contract);

    let roles: Vec<Role> = messages.iter().map(|message| message.role).collect();
    let mut seen = std::collections::HashSet::<String>::new();
    let mut kept_below_floor = 0;
    let mut out = Vec::with_capacity(messages.len());
    for (index, message) in messages.into_iter().enumerate() {
        let keep = if message.role != Role::System {
            true
        } else if is_contract(&message) {
            index >= floor || Some(index) == newest_contract_below_floor
        } else {
            let follows_assistant = index > 0 && roles[index - 1] == Role::Assistant;
            seen.insert(message.content.clone()) || follows_assistant
        };
        if keep {
            if index < floor {
                kept_below_floor += 1;
            }
            out.push(message);
        }
    }
    (out, kept_below_floor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contents(messages: &[Message]) -> Vec<&str> {
        messages.iter().map(|m| m.content.as_str()).collect()
    }

    #[test]
    fn a_repeated_end_of_turn_nudge_still_reaches_the_model() {
        let review = "Before finishing, do ONE final review.";
        let messages = vec![
            Message::user("first request"),
            Message::assistant("first answer"),
            Message::system(review),
            Message::assistant("review done"),
            Message::user("second request"),
            Message::assistant("second answer"),
            Message::system(review),
        ];

        let view = normalize_system_context(messages, 0).0;

        assert_eq!(
            contents(&view).iter().filter(|c| **c == review).count(),
            2,
            "the second turn's review nudge was deduplicated away: {:?}",
            contents(&view)
        );
        assert_eq!(view.last().unwrap().content, review);
    }

    /// The rule may not look at what comes next. A repeat right after a reply used to be kept or
    /// dropped depending on whether a user prompt followed it, so appending that prompt rewrote a
    /// message the provider had already cached. Now the repeat after a reply is kept either way.
    #[test]
    fn a_repeat_after_a_reply_is_kept_whatever_follows_it() {
        let guidance = "standing workflow guidance";
        let mut messages = vec![
            Message::user("one"),
            Message::assistant("answer"),
            Message::system(guidance),
            Message::user("two"),
            Message::assistant("answer two"),
            Message::system(guidance),
        ];
        let before = normalize_system_context(messages.clone(), 0).0;
        messages.push(Message::user("three"));
        let after = normalize_system_context(messages, 0).0;

        assert_eq!(
            contents(&after).iter().filter(|c| **c == guidance).count(),
            2
        );
        assert_eq!(contents(&before), contents(&after[..before.len()]));
    }

    #[test]
    fn a_superseded_contract_is_only_dropped_below_the_floor() {
        let messages = vec![
            Message::system("Turn contract: this request is explicitly read-only."),
            Message::user("look"),
            Message::assistant("looked"),
            Message::system("Turn contract: supersedes earlier turn contracts — build it."),
            Message::user("build"),
        ];
        let (in_place, below) = normalize_system_context(messages.clone(), 0);
        assert_eq!(in_place.len(), 5);
        assert_eq!(below, 0);

        // Both contracts below the floor: only the newest survives there.
        let (pruned, below) = normalize_system_context(messages.clone(), 4);
        assert_eq!(pruned.len(), 4);
        assert_eq!(below, 3);
        assert!(!contents(&pruned).iter().any(|c| c.contains("read-only")));

        // The newer contract above the floor cannot reach back and drop the older one.
        let (split, below) = normalize_system_context(messages, 3);
        assert_eq!(split.len(), 5);
        assert_eq!(below, 3);
    }

    #[test]
    fn standing_guidance_not_preceded_by_an_assistant_is_still_deduplicated() {
        let guidance = "[lsp diagnostics] unused import";
        let messages = vec![
            Message::user("one"),
            Message::system(guidance),
            Message::user("two"),
            Message::system(guidance),
            Message::user("three"),
        ];

        let view = normalize_system_context(messages, 0).0;

        assert_eq!(
            contents(&view).iter().filter(|c| **c == guidance).count(),
            1
        );
    }

    #[test]
    fn dropping_a_duplicate_depends_only_on_earlier_messages() {
        let nudge = "You stopped after announcing your next step.";
        let mut messages = vec![
            Message::user("a"),
            Message::assistant("b"),
            Message::system(nudge),
        ];
        let before = normalize_system_context(messages.clone(), 0).0;

        messages.push(Message::assistant("c"));
        messages.push(Message::system(nudge));
        messages.push(Message::user("d"));
        let after = normalize_system_context(messages, 0).0;

        for (index, sent) in before.iter().enumerate() {
            assert_eq!(sent.content, after[index].content);
        }
    }
}
