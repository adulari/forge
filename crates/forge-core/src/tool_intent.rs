//! Which turns are shown tools, and when a model that was shown none has proved it needs them.
//!
//! A trivial-tier turn is sent without tools unless the prompt shows a workspace or external-action
//! intent: a small model handed a tool list for "reply with exactly X" tends to answer with a call.
//! The heuristic is a keyword list, so it misses questions that only name a file ("which function
//! in `a/b.py` validates X") and the model, with nothing to call, writes the call as text. Two
//! backstops: a prompt that names a source file is let through, and a model that writes a tool call
//! as text gets the tools on its next request.

use super::*;

const INTENT_WORDS: &[&str] = &[
    "read ",
    "inspect ",
    "search ",
    "find ",
    "grep",
    "rg ",
    "file",
    "directory",
    "repo",
    "code",
    "test",
    "build",
    "compile",
    "run ",
    "execute",
    "shell",
    "command",
    "git",
    "commit",
    "diff",
    "write",
    "edit",
    "create",
    "delete",
    "implement",
    "fix ",
    "debug",
    "diagnose",
    "review",
    "refactor",
    "install",
    "web",
    "http",
    "url",
    "browser",
    "fetch",
    "download",
    "mcp",
    "database",
    "query",
    "rename",
    "replace",
    "add ",
    "update",
    "remove",
    "change",
    "summarize",
    "readme",
    "everywhere",
    "across",
    "subagent",
    "sub-agent",
    "spawn",
    "delegate",
    "parallel",
    "workflow",
];

/// Extensions that make a bare word a file name (`limits.py`, `src/lib.rs`, `Cargo.toml`).
const FILE_EXTENSIONS: &[&str] = &[
    "rs", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "go", "mod", "sum", "java", "kt", "kts",
    "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "sh", "bash", "zsh", "toml", "yaml",
    "yml", "json", "md", "txt", "cfg", "ini", "conf", "lock", "sql", "html", "css", "scss", "vue",
    "svelte", "lua", "ex", "exs", "hs", "ml", "scala", "dart", "zig", "nix", "gradle", "xml",
];

fn is_file_name(token: &str) -> bool {
    let name = token.trim_matches(['.', '-']);
    let name = name.rsplit('/').next().unwrap_or(name);
    name.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && FILE_EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
    })
}

/// Whether a trivial-tier prompt should still be shown tools.
pub(crate) fn prompt_wants_tools(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    INTENT_WORDS.iter().any(|intent| prompt.contains(intent))
        || prompt
            .split(|c: char| c.is_whitespace() || "`\"'()[]{},;:<>?!".contains(c))
            .any(is_file_name)
}

impl Session {
    /// The tool surface for one model request. A turn that started without tools gets them from
    /// the step after the model showed it needs them (see `tools_advertised_once`).
    pub(crate) fn specs_for_step<'a>(
        &self,
        advertised: &'a [ToolSpec],
    ) -> std::borrow::Cow<'a, [ToolSpec]> {
        if advertised.is_empty() && self.tools_advertised_once {
            std::borrow::Cow::Owned(self.tool_specs())
        } else {
            std::borrow::Cow::Borrowed(advertised)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_prompt_that_names_a_source_file_is_shown_tools() {
        for prompt in [
            "What does `pkg/limits.py` do with a negative interval?",
            "what does src/lib.rs export",
            "explain (Cargo.toml) to me",
            "Why does main.go:12 panic?",
            "look at ./scripts/run.SH.",
        ] {
            assert!(prompt_wants_tools(prompt), "{prompt}");
        }
    }

    #[test]
    fn chat_that_only_looks_like_a_path_is_not() {
        for prompt in [
            "Reply exactly: daemon-stability-check.",
            "should I answer yes/no? e.g. maybe",
            "what is 3.14 times 2",
            "is it v1.2 or v2.0",
            "tell me a joke",
            "ends with a dot. and again.",
        ] {
            assert!(!prompt_wants_tools(prompt), "{prompt}");
        }
    }

    #[test]
    fn the_keyword_intents_still_work() {
        for prompt in ["summarize README", "fix the build", "run the tests"] {
            assert!(prompt_wants_tools(prompt), "{prompt}");
        }
    }
}
