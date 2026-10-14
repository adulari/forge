//! `run_code` over the deferred registry: a script finds a tool the lean surface hides with
//! `tools.$search` and calls it by name, and the inner call meets the same permission gate and
//! mutation accounting as a plain `tool_call` (docs/features/codemode.md).

use super::*;

const NOTEBOOK: &str = r#"{"cells":[{"cell_type":"code","source":["print(1)"],"metadata":{},"outputs":[],"execution_count":null}],"metadata":{},"nbformat":4,"nbformat_minor":5}"#;

fn notebook_script(path: &std::path::Path) -> String {
    format!(
        "const hits = await tools.$search('notebook_edit');\n\
         try {{\n\
           await tools.tool_call({{name: hits[0].name, arguments: {{path: {p:?}, cell: 0, source: 'print(2)'}}}});\n\
           return hits[0].name + ' wrote';\n\
         }} catch (e) {{ return hits[0].name + ' refused: ' + e.message; }}",
        p = path.to_str().unwrap()
    )
}

#[test]
fn run_code_stays_advertised_while_the_tool_it_reaches_is_deferred() {
    let dir = workspace("deferred-surface");
    let session = session_in(&dir, PermissionMode::AcceptEdits, Approver::default());
    let names: Vec<String> = session.tool_specs().into_iter().map(|s| s.name).collect();
    assert!(
        names
            .iter()
            .any(|n| n == crate::deferred_tools::TOOL_SEARCH),
        "lean surface is on: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == codemode::RUN_CODE_TOOL),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n == "notebook_edit"), "{names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn a_deferred_write_from_a_script_is_denied_in_plan_mode_and_mutates_nothing() {
    let dir = workspace("deferred-plan");
    let path = dir.join("nb.ipynb");
    std::fs::write(&path, NOTEBOOK).unwrap();
    let mut session = session_in(&dir, PermissionMode::Plan, Approver::default());
    let ledgers = Ledgers::new();

    let (msg_id, result) = step(
        &mut session,
        &ledgers,
        run_code_call(&notebook_script(&path), None),
    )
    .await;

    assert!(result.starts_with("notebook_edit refused: "), "{result}");
    assert!(result.contains("denied"), "{result}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), NOTEBOOK);
    assert_eq!(
        ledgers.mutations_ran.load(Ordering::SeqCst),
        0,
        "a denied write is not a mutation"
    );
    assert_eq!(
        ledgers.tools_ran.load(Ordering::SeqCst),
        2,
        "the denied inner call still counts as a call ($search does not)"
    );
    let rows = audit(&session, &msg_id);
    assert_eq!(rows[0].0, "notebook_edit", "audited under the real name");
    assert_ne!(rows[0].1, "ok", "{rows:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn an_allowed_deferred_write_from_a_script_counts_as_a_mutation() {
    let dir = workspace("deferred-write");
    let path = dir.join("nb.ipynb");
    std::fs::write(&path, NOTEBOOK).unwrap();
    let mut session = session_in(&dir, PermissionMode::Bypass, Approver::default());
    let ledgers = Ledgers::new();

    let (_, result) = step(
        &mut session,
        &ledgers,
        run_code_call(&notebook_script(&path), None),
    )
    .await;

    assert!(result.starts_with("notebook_edit wrote"), "{result}");
    assert!(std::fs::read_to_string(&path).unwrap().contains("print(2)"));
    assert_eq!(
        ledgers.mutations_ran.load(Ordering::SeqCst),
        1,
        "phantom-edit gate sees the deferred write"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
