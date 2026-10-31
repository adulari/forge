/// Every file path a call targets: the primary path arg, batched `paths[]`, multi-file
/// `edits[].path`, and the targets named inside an `apply_patch` body (Codex envelope headers and
/// unified-diff `---`/`+++` headers). Relative patch targets are joined onto `cwd` when given.
/// Permission rules and auto-mode risk must judge all of these, not just the first path key.
pub fn all_path_args(args: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(p) = crate::extract_path_arg(args) {
        out.push(p.to_string());
        // `read_file` takes `path: "key.pem:raw"` too; the rules must judge the file.
        out.extend(strip_line_range(p).map(String::from));
    }
    if let Some(list) = args.get("paths").and_then(|v| v.as_array()) {
        for entry in list.iter().filter_map(|v| v.as_str()) {
            out.push(entry.to_string());
            // `read_file` takes `src/lib.rs:120-180`; the rules must judge the file, not the span.
            out.extend(strip_line_range(entry).map(String::from));
        }
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

/// The path of a `path:SPEC` entry, or `None` when it carries no selector. Every form
/// `read_file` accepts counts: `:12`, `:5-16`, `:5-16,960-973`, `:40-`, `:-60` and `:raw`.
pub fn strip_line_range(entry: &str) -> Option<&str> {
    let (path, spec) = entry.rsplit_once(':')?;
    (!path.is_empty() && is_selector(spec)).then_some(path)
}

fn is_selector(spec: &str) -> bool {
    if spec == "raw" {
        return true;
    }
    if let Some(n) = spec.strip_prefix('-') {
        return n.parse::<usize>().is_ok_and(|n| n >= 1);
    }
    spec.split(',').all(|part| {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let Some(start) = a.parse::<usize>().ok().filter(|s| *s >= 1) else {
            return false;
        };
        b.is_empty() || b.parse::<usize>().is_ok_and(|end| end >= start)
    })
}

/// Whether `path` names a dotenv *template* (`.env.example`, `.env.local.sample`, ...): a file
/// meant to be committed, which holds placeholders and no secrets. The secret-file rules cover
/// every `.env.*`, and this is the one exception they make.
pub fn is_dotenv_template(path: &str) -> bool {
    let base = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    base.starts_with(".env.")
        && [".example", ".sample", ".template", ".dist"]
            .iter()
            .any(|suffix| base.ends_with(suffix))
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

    #[test]
    fn a_line_range_entry_also_names_its_file() {
        let args = serde_json::json!({ "paths": ["src/a.rs:2-3", ".env:1", "notes.md", "t:0-1"] });
        assert_eq!(
            all_path_args(&args),
            [
                "src/a.rs:2-3",
                "src/a.rs",
                ".env:1",
                ".env",
                "notes.md",
                "t:0-1"
            ]
        );
        assert_eq!(strip_line_range("a/b.rs:10-20"), Some("a/b.rs"));
        assert_eq!(strip_line_range("a/b.rs:7"), Some("a/b.rs"));
        assert_eq!(strip_line_range("a/b.rs:20-10"), None);
        assert_eq!(strip_line_range("a/b.rs"), None);
        assert_eq!(strip_line_range(":3"), None);
    }

    #[test]
    fn every_read_selector_form_names_its_file_in_path_and_paths() {
        for spec in ["raw", "-5", "1-2,3-4", "40-", "12", "5-16"] {
            let entry = format!(".env:{spec}");
            assert_eq!(strip_line_range(&entry), Some(".env"), "{entry}");
            assert!(all_path_args(&serde_json::json!({ "path": entry })).contains(&".env".into()));
            assert!(
                all_path_args(&serde_json::json!({ "paths": [entry] })).contains(&".env".into())
            );
        }
        for not in [
            ".env:rawx",
            ".env:-0",
            ".env:3-1",
            ".env:1,",
            ".env:",
            "C:\\x",
        ] {
            assert_eq!(strip_line_range(not), None, "{not}");
        }
    }

    #[test]
    fn only_example_style_dotenv_files_are_templates() {
        for template in [
            ".env.example",
            "./.env.example",
            "config/.env.local.sample",
            "C:\\app\\.env.template",
            "/srv/app/.ENV.DIST",
        ] {
            assert!(is_dotenv_template(template), "{template}");
        }
        for secret in [
            ".env",
            ".env.local",
            ".env.production",
            ".env.example.local",
            ".env.example.bak",
            "example.env",
            "env.example",
            "config/.env.example/.env",
        ] {
            assert!(!is_dotenv_template(secret), "{secret}");
        }
    }
}
