//! One-line skill summaries for the `use_skill` listing.

const TRIGGERS: [&str; 3] = ["use when", "use this when", "use whenever"];
const TRIGGER_CHARS: usize = 45;

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let at_boundary = text.chars().nth(max).is_some_and(char::is_whitespace);
    let head = if at_boundary {
        cut.as_str()
    } else {
        cut.rsplit_once(char::is_whitespace)
            .map_or(cut.as_str(), |(head, _)| head)
    };
    format!("{}…", head.trim_end_matches([' ', ',', ';', ':', '.']))
}

/// Clip `description` to about `max` chars on a word boundary. When the clip would drop a
/// "Use when ..." clause, that clause rides along (shortened) so the model can still see when to
/// pick the skill.
pub fn summarize_description(description: &str, max: usize) -> String {
    let description = description.trim();
    if description.chars().count() <= max {
        return description.to_string();
    }
    let lower = description.to_ascii_lowercase();
    let trigger = TRIGGERS
        .iter()
        .filter_map(|t| lower.find(t))
        .min()
        .filter(|&at| description.is_char_boundary(at));
    match trigger {
        Some(at) if at > 0 => {
            let head = clip(
                description[..at].trim_end(),
                max.saturating_sub(TRIGGER_CHARS).max(20),
            );
            format!("{head} {}", clip(&description[at..], TRIGGER_CHARS))
        }
        _ => clip(description, max),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_descriptions_pass_through() {
        assert_eq!(summarize_description("a demo skill", 60), "a demo skill");
    }

    #[test]
    fn clips_on_a_word_boundary() {
        let out = summarize_description(&"word ".repeat(40), 60);
        assert!(out.ends_with("word…"), "{out}");
        assert!(out.chars().count() <= 61);
    }

    #[test]
    fn the_use_when_clause_survives() {
        let desc = "A thorough multi-stage release assistant for production services and more. \
                    Use when the user asks to ship, deploy, release anything at all.";
        let out = summarize_description(desc, 60);
        assert!(out.contains("Use when the user asks"), "{out}");
        assert!(!out.contains("servi…ces"), "{out}");
    }

    #[test]
    fn unbroken_text_is_still_cut() {
        assert_eq!(
            summarize_description(&"x".repeat(400), 60).chars().count(),
            61
        );
    }
}
