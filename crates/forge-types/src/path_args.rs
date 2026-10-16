/// Every file path a call targets: the primary path arg, batched `paths[]`, multi-file
/// `edits[].path`, and the targets named inside an `apply_patch` body (Codex envelope headers and
/// unified-diff `---`/`+++` headers). Relative patch targets are joined onto `cwd` when given.
/// Permission rules and auto-mode risk must judge all of these, not just the first path key.
pub fn all_path_args(args: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = crate::extract_path_arg(args) {
        out.push(p.to_string());
    }
    if let Some(list) = args.get("paths").and_then(|v| v.as_array()) {
        out.extend(list.iter().filter_map(|v| v.as_str()).map(String::from));
    }
    if let Some(edits) = args.get("edits").and_then(|v| v.as_array()) {
        out.extend(
            edits
                .iter()
                .filter_map(|e| e.get("path").and_then(|v| v.as_str()))
                .map(String::from),
        );
    }
    if let Some(patch) = args.get("patch").and_then(|v| v.as_str()) {
        let cwd = args.get("cwd").and_then(|v| v.as_str());
        for target in patch_targets(patch) {
            let rooted = match cwd {
                Some(c) if !target.starts_with('/') && !target.starts_with('~') => {
                    format!("{}/{target}", c.trim_end_matches('/'))
                }
                _ => target,
            };
            out.push(rooted);
        }
    }
    out
}

fn patch_targets(patch: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in patch.lines() {
        let target = [
            "*** Add File: ",
            "*** Update File: ",
            "*** Delete File: ",
            "*** Move to: ",
        ]
        .iter()
        .find_map(|h| line.strip_prefix(h))
        .or_else(|| {
            line.strip_prefix("+++ ")
                .or_else(|| line.strip_prefix("--- "))
                .map(|t| t.split('\t').next().unwrap_or(t))
                .map(|t| {
                    t.strip_prefix("a/")
                        .or_else(|| t.strip_prefix("b/"))
                        .unwrap_or(t)
                })
        });
        if let Some(t) = target.map(str::trim) {
            if !t.is_empty() && t != "/dev/null" {
                out.push(t.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_path_args_sees_lists_edits_and_patch_targets() {
        let args = serde_json::json!({
            "path": "a",
            "paths": ["b"],
            "edits": [{ "path": "c" }],
            "cwd": "../out",
            "patch": "*** Begin Patch\n*** Add File: d.txt\n+x\n*** End Patch\n--- a/e\n+++ b/f\n--- /dev/null\n"
        });
        assert_eq!(
            all_path_args(&args),
            ["a", "b", "c", "../out/d.txt", "../out/e", "../out/f"]
        );
    }
}
