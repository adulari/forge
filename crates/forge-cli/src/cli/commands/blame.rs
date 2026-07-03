use std::path::Path;

use anyhow::{Context, Result};

use crate::*;

/// `forge blame <file>` — whole-file summary; `forge blame <file> --line N` — full provenance
/// card for one line; `--json` emits the whole-file summary as machine-readable JSON.
/// docs/features/forge-blame.md.
pub(crate) fn blame_cmd(file: &str, line: Option<usize>, json: bool) -> Result<()> {
    let store = open_store()?;
    let current = std::fs::read_to_string(file).with_context(|| format!("reading {file}"))?;

    // Resolve to the same form recorded edits are matched against: canonicalize when the file
    // exists on disk (handles `..`, symlinks, relative-vs-absolute spelling); otherwise fall back
    // to cwd-joined-but-uncanonicalized, same fallback `blame::matching_edits` itself applies.
    let target = Path::new(file);
    let target = target.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|cwd| cwd.join(target))
            .unwrap_or_else(|_| target.to_path_buf())
    });

    // Narrow the store query with the plain file name (the one suffix guaranteed to survive
    // regardless of which cwd a past session ran `write_file`/`edit_file` from); the precise
    // per-row path resolution below is what actually decides "is this the same file".
    let suffix = target.file_name().and_then(|n| n.to_str()).unwrap_or(file);
    let candidates = store
        .file_edits(suffix)
        .with_context(|| format!("loading recorded edits for {file}"))?;
    let edits: Vec<forge_store::FileEditRow> = blame::matching_edits(&target, &candidates)
        .into_iter()
        .cloned()
        .collect();

    let attributions = blame::attribute_lines(&current, &edits);
    let now = chrono::Utc::now().timestamp();

    if let Some(line_no) = line {
        let Some(attr) = attributions.get(line_no.saturating_sub(1)) else {
            anyhow::bail!(
                "line {line_no} is out of range — {file} has {} lines",
                attributions.len()
            );
        };
        let turn = match &attr.session_id {
            Some(session_id) => store
                .turn_context(session_id, attr.seq.unwrap_or(0))
                .with_context(|| format!("loading turn context for session {session_id}"))?,
            None => forge_store::TurnContext::default(),
        };
        print!("{}", blame::render_why(attr, &turn, now));
    } else if json {
        println!("{}", blame::render_json(&attributions));
    } else {
        print!("{}", blame::render_blame(&attributions, now));
    }
    Ok(())
}
