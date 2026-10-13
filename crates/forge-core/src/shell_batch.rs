//! Core-side reading of a `shell` batch result (`commands: [...]`, see forge-tools'
//! `shell/batch.rs`): which entry failed, so the shell diagnosis explains that one command and its
//! own output instead of the whole batch.

/// The command and output to diagnose for a failed shell call: the call's single command and the
/// whole result, or for a batch the first failing entry and its section.
pub(crate) fn failing_command(args: &serde_json::Value, result: &str) -> Option<(String, String)> {
    if let Some(command) = args.get("command").and_then(serde_json::Value::as_str) {
        return Some((command.to_string(), result.to_string()));
    }
    args.get("commands")?;
    let mut sections = result.split("\n\n── [").skip(1);
    sections.find_map(|section| {
        let (head, body) = section.split_once('\n')?;
        let command = head.split_once("] ")?.1;
        let failed = !body.starts_with("shell: exit 0 ");
        failed.then(|| (command.to_string(), body.to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_single_command_diagnoses_the_whole_result() {
        let args = serde_json::json!({ "command": "make" });
        assert_eq!(
            failing_command(&args, "shell: exit 2 in 5ms\nboom"),
            Some(("make".into(), "shell: exit 2 in 5ms\nboom".into()))
        );
    }

    #[test]
    fn a_batch_diagnoses_its_first_failing_entry_only() {
        let args = serde_json::json!({ "commands": ["echo ok", "cargo test", "echo later"] });
        let result = "shell: exit 101 in 9ms (2/3 ok)\n\n── [1] echo ok\nshell: exit 0 in 1ms\nok\
                      \n\n── [2] cargo test\nshell: exit 101 in 7ms\ntest failed\
                      \n\n── [3] echo later\nshell: exit 0 in 1ms\nlater";
        assert_eq!(
            failing_command(&args, result),
            Some((
                "cargo test".into(),
                "shell: exit 101 in 7ms\ntest failed".into()
            ))
        );
    }
}
