//! `/stash` argument parsing.

/// What `/stash` does with the parked drafts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StashOp {
    Push,
    Pop,
    List,
    Clear,
}

impl StashOp {
    /// A bare `/stash` pops when something is parked; the draft itself is on the composer's line
    /// when the command is typed, so it cannot be pushed from there — `push` exists for the palette.
    pub fn parse(arg: &str) -> Self {
        match arg.trim().to_lowercase().as_str() {
            "push" | "save" => Self::Push,
            "list" | "ls" => Self::List,
            "clear" => Self::Clear,
            _ => Self::Pop,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_verbs_and_defaults_to_pop() {
        assert_eq!(StashOp::parse(""), StashOp::Pop);
        assert_eq!(StashOp::parse("pop"), StashOp::Pop);
        assert_eq!(StashOp::parse("LIST"), StashOp::List);
        assert_eq!(StashOp::parse("clear"), StashOp::Clear);
        assert_eq!(StashOp::parse("push"), StashOp::Push);
    }
}
