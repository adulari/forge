//! Recognises a message another agent session sent, as framed by the fleet delivery path
//! (`forge_core::fleet::peer_message_text`), so the transcript can show it as a peer message
//! rather than as something the user typed. The framing lives in forge-core; this crate only
//! knows the shape: `[message from <who>] <notice>\n---\n<body>`, or the short
//! `[message from <who>] <body>` for non-agent senders (`cli`, `dispatch`).

/// `(who, body)` when `line` is a delivered fleet message.
pub(crate) fn parse(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("[message from ")?;
    let (who, tail) = rest.split_once("] ")?;
    if who.is_empty() || who.contains('\n') {
        return None;
    }
    let body = tail.split_once("\n---\n").map_or(tail, |(_, body)| body);
    Some((who.to_string(), body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn agent_messages_drop_the_boilerplate_notice_and_keep_the_body() {
        let framed = "[message from session \"alpha\" (3fa91c2d)] This is a peer message.\n---\nplease check the build\nthanks";
        assert_eq!(
            parse(framed),
            Some((
                "session \"alpha\" (3fa91c2d)".to_string(),
                "please check the build\nthanks".to_string()
            ))
        );
    }

    #[test]
    fn short_form_and_ordinary_prompts() {
        assert_eq!(
            parse("[message from cli] hello"),
            Some(("cli".to_string(), "hello".to_string()))
        );
        assert_eq!(parse("fix the bug"), None);
        assert_eq!(parse("[message from ] x"), None);
    }
}
