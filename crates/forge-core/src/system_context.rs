//! The provider-facing view of system-role messages in a transcript.

use forge_types::{Message, Role};

const TURN_CONTRACT_PREFIX: &str = "Turn contract:";

/// Derive the provider's system-context view without rewriting the persisted transcript. A turn
/// contract is scoped to one turn, so only the newest contract may remain authoritative.
///
/// Exact repeated system guidance is standing context, not cumulative evidence, so only one full
/// copy is sent — but WHICH copy matters more than it looks. Prompt caches key on a PREFIX, so
/// keeping the newest copy deletes the previously-kept one from the middle of the prompt and
/// discards every cached token after it. One live session showed 344 exact-duplicate system
/// messages, mostly re-emitted `[lsp diagnostics]` blocks: 344 invalidations of a ~50k-token
/// prompt, each to save ~380 chars. Keeping the FIRST copy is just as bounded and costs nothing,
/// because whether a message is dropped then depends only on the messages before it — so a
/// transcript that grows never rewrites what the provider has already cached.
///
/// A system message that directly follows an assistant message and is answered by the model (the
/// next message is not a user prompt) is NOT standing context: it is the harness reacting to what
/// the model just said (the end-of-turn review, the "prove it" completion gate, a stalled-step
/// nudge). Its text is identical every turn, so deduplicating it deleted the reaction from the
/// request entirely — the model was re-driven with nothing new to act on, and a persistent CLI
/// bridge answered "nothing new is requested" as the turn's final reply. Guidance that sits
/// between turns, in front of the next user prompt, remains standing context.
pub(crate) fn normalize_system_context(messages: Vec<Message>) -> Vec<Message> {
    let newest_contract = messages.iter().rposition(|message| {
        message.role == Role::System
            && message
                .content
                .trim_start()
                .starts_with(TURN_CONTRACT_PREFIX)
    });

    let roles: Vec<Role> = messages.iter().map(|message| message.role).collect();
    let mut seen = std::collections::HashSet::<String>::new();
    messages
        .into_iter()
        .enumerate()
        .filter_map(|(index, message)| {
            let follows_assistant = index > 0 && roles[index - 1] == Role::Assistant;
            let precedes_user = roles.get(index + 1) == Some(&Role::User);
            let reaction = follows_assistant && !precedes_user;
            if message.role != Role::System {
                return Some(message);
            }
            if message
                .content
                .trim_start()
                .starts_with(TURN_CONTRACT_PREFIX)
            {
                return (Some(index) == newest_contract).then_some(message);
            }
            let first_copy = seen.insert(message.content.clone());
            (first_copy || reaction).then_some(message)
        })
        .collect()
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

        let view = normalize_system_context(messages);

        assert_eq!(
            contents(&view).iter().filter(|c| **c == review).count(),
            2,
            "the second turn's review nudge was deduplicated away: {:?}",
            contents(&view)
        );
        assert_eq!(view.last().unwrap().content, review);
    }

    #[test]
    fn guidance_between_turns_is_still_deduplicated() {
        let guidance = "standing workflow guidance";
        let messages = vec![
            Message::user("one"),
            Message::assistant("answer"),
            Message::system(guidance),
            Message::user("two"),
            Message::assistant("answer two"),
            Message::system(guidance),
            Message::user("three"),
        ];

        let view = normalize_system_context(messages);

        assert_eq!(
            contents(&view).iter().filter(|c| **c == guidance).count(),
            1
        );
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

        let view = normalize_system_context(messages);

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
        let before = normalize_system_context(messages.clone());

        messages.push(Message::assistant("c"));
        messages.push(Message::system(nudge));
        messages.push(Message::user("d"));
        let after = normalize_system_context(messages);

        for (index, sent) in before.iter().enumerate() {
            assert_eq!(sent.content, after[index].content);
        }
    }
}
