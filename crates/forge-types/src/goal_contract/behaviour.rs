//! The second assertion kind of a goal contract: a behaviour assertion
//! (docs/features/goal-contract.md, "Behaviour assertions (QA)"). Instead of a read-only shell
//! check it names a thing to start, a probe that uses it and the text the probe must show. The
//! QA runner in forge-core executes it; this module is the data and its tolerant parsing.

use super::{clip, text_of, COMMAND_CHARS, TEXT_CHARS};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Behaviour {
    /// Starts the thing under test. Runs in the workspace with `FORGE_QA_PORT` (a free port) and
    /// `FORGE_QA_DIR` (a scratch directory) set.
    pub run: String,
    /// Uses it: `GET <url>`, `screenshot <url>`, or a shell command.
    pub probe: String,
    /// Text the probe's output must contain.
    #[serde(default)]
    pub expect: String,
    /// Always runs afterwards, before the process group is killed.
    #[serde(default)]
    pub cleanup: String,
    /// `port` / `port:N` / `line:<text>`: wait for this before probing once. Empty: retry the
    /// probe until it answers.
    #[serde(default)]
    pub ready: String,
}

impl Behaviour {
    /// The one-line description the surfaces show in place of a read-only command.
    pub fn summary(&self) -> String {
        format!("behaviour: run `{}`, probe `{}`", self.run, self.probe)
    }

    /// Parse the `behaviour` object of a drafted assertion. `None` without a run and a probe.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        let map = value.as_object()?;
        let get =
            |keys: &[&str], max: usize| clip(&text_of(keys.iter().find_map(|k| map.get(*k))), max);
        let run = get(&["run", "start", "command"], COMMAND_CHARS);
        let probe = get(&["probe", "request", "check"], COMMAND_CHARS);
        if run.is_empty() || probe.is_empty() {
            return None;
        }
        Some(Self {
            run,
            probe,
            expect: get(&["expect", "expected", "contains"], TEXT_CHARS),
            cleanup: get(&["cleanup", "stop"], COMMAND_CHARS),
            ready: get(&["ready", "wait_for"], TEXT_CHARS),
        })
    }
}
