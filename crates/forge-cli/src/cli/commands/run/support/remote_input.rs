use super::*;

/// The next input event for the key loop: remote-injected keys first (so a remote overlay
/// commit — cursor move + synthesized Enter — is never interleaved by local typing), then the
/// terminal's own events. Remote keys become plain [`forge_tui::InputEvent::Key`]s here, so from
/// this point on they are indistinguishable from local keystrokes — the ONE code path both take.
pub(crate) fn next_input_event(
    remote_keys: &mut std::collections::VecDeque<forge_tui::KeyKind>,
    tui: &mut forge_tui::Tui,
) -> Result<Option<forge_tui::InputEvent>> {
    if let Some(k) = remote_keys.pop_front() {
        return Ok(Some(forge_tui::InputEvent::Key(k)));
    }
    tui.poll_event().context("reading input")
}

/// True when any modal surface owns the keyboard — the same set `remote_overlay()` projects.
pub(crate) fn any_remote_modal_open(app: &forge_tui::App) -> bool {
    app.workflow.open
        || app.config_editor.open
        || app.command_center.open
        || app.palette.open
        || app.usage_overlay.open
        || app.mesh_overlay.open
        || app.at_picker.open
        || app.picker.open
}

/// A remote overlay verb, decoded from [`remote::RemoteInput`] by the drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteOverlayOp {
    /// Move the cursor onto the row with this id, then commit it (synthesized Enter).
    Select(String),
    /// Move the cursor by this many rows (negative = up), as repeated ↑/↓ keys.
    Nav(i32),
    /// Replace the overlay's filter/query text (or the value being edited, for free-text).
    Filter(String),
    /// Close the overlay (Esc) — a no-op when nothing modal is open.
    Cancel,
}

/// Apply a remote overlay verb to the TOP-MOST open overlay (same precedence as
/// `App::remote_overlay`) and return the keystrokes to inject through the normal key path.
/// Select = set the cursor to the row with that id, then Enter — so a remotely committed picker
/// produces the identical `DispatchOutcome` handling a local Enter does. All mutations here are
/// cursor/filter state only; every side effect still happens in the shared key path.
pub(crate) fn apply_overlay_input(
    app: &mut forge_tui::App,
    op: RemoteOverlayOp,
) -> Vec<forge_tui::KeyKind> {
    use forge_tui::KeyKind as K;
    match op {
        RemoteOverlayOp::Cancel => {
            if any_remote_modal_open(app) {
                vec![K::Esc]
            } else {
                Vec::new()
            }
        }
        RemoteOverlayOp::Nav(delta) => {
            if !any_remote_modal_open(app) || delta == 0 {
                return Vec::new();
            }
            let key = if delta < 0 { K::Up } else { K::Down };
            // Bounded: a hostile frame can't queue an unbounded key storm.
            vec![key; delta.unsigned_abs().min(100) as usize]
        }
        RemoteOverlayOp::Filter(text) => {
            if app.workflow.open || app.usage_overlay.open || app.mesh_overlay.open {
                // Informational overlays have no filter.
            } else if app.config_editor.open {
                if app.config_editor.editing.is_some() {
                    app.config_editor.editing = Some(text);
                } else {
                    app.config_editor.filter = text;
                    app.config_editor.selected = 0;
                }
            } else if app.command_center.open {
                app.command_center.query = text;
                app.command_center.selected = 0;
                app.command_center.clamp(&app.palette.extra);
            } else if app.palette.open {
                // Mirror local typing: the palette query IS the input line's slash token.
                app.input = format!("/{text}");
                app.input_cursor = app.input.len();
                app.palette.query = text;
                app.palette.selected = 0;
                app.palette.clamp();
            } else if app.at_picker.open {
                app.at_picker.query = text;
                app.at_picker.selected = 0;
            } else if app.picker.open {
                app.picker.query = text;
                app.picker.selected = 0;
                app.picker.clamp();
            }
            Vec::new()
        }
        RemoteOverlayOp::Select(id) => {
            if app.workflow.open {
                if let Some(idx) = app.workflow.rows.iter().position(|r| r.id == id) {
                    app.workflow.selected = idx;
                }
                Vec::new() // Enter would zoom a transcript only the host can see
            } else if app.config_editor.open {
                if app.config_editor.editing.is_some() {
                    return Vec::new(); // committing the edit is the free-text box's job
                }
                let matches = app.config_editor.matches();
                if let Some(pos) = matches
                    .iter()
                    .position(|&i| app.config_editor.rows[i].path == id)
                {
                    app.config_editor.selected = pos;
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.command_center.open {
                let entries = app.command_center.matches(&app.palette.extra);
                if let Some(idx) = entries.iter().position(|entry| entry.name == id) {
                    app.command_center.selected = idx;
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.palette.open {
                let names: Vec<String> =
                    app.palette.matches().into_iter().map(|e| e.name).collect();
                if let Some(idx) = names.iter().position(|n| *n == id) {
                    app.palette.selected = idx;
                    // A leading `/command` input line is what makes the palette's Enter
                    // dispatch (vs. accept-in-place) — materialize the pick exactly as typed.
                    app.input = format!("/{id}");
                    app.input_cursor = app.input.len();
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.usage_overlay.open {
                Vec::new() // informational — rows aren't selectable
            } else if app.mesh_overlay.open {
                if let Some(idx) = app
                    .mesh_overlay
                    .candidates
                    .iter()
                    .position(|c| c.model == id)
                {
                    app.mesh_overlay.cursor = idx;
                }
                Vec::new() // browsing highlight only, same as local ↑/↓
            } else if app.at_picker.open {
                if let Some(idx) = app.at_picker.matches().iter().position(|p| **p == id) {
                    app.at_picker.selected = idx;
                    return vec![K::Enter];
                }
                Vec::new()
            } else if app.picker.open {
                if let Some(idx) = app.picker.matches().iter().position(|r| r.id == id) {
                    app.picker.selected = idx;
                    return vec![K::Enter];
                }
                Vec::new()
            } else {
                Vec::new()
            }
        }
    }
}

/// Append a remote-facing notice (`Snapshot::notes`), keeping the ring bounded. These are state,
/// not events — `watch` coalescing can drop intermediate snapshots, so a note must survive until
/// the page has had a chance to render it.
pub(crate) fn push_remote_note(notes: &mut Vec<String>, msg: &str) {
    const MAX_REMOTE_NOTES: usize = 8;
    notes.push(msg.to_string());
    while notes.len() > MAX_REMOTE_NOTES {
        notes.remove(0);
    }
}

/// Prefix a remote prompt with the pending uploaded-text-file mentions (drained), so
/// `expand_at_files` inlines their contents exactly like a locally typed `@path`.
pub(crate) fn prepend_attach_mentions(mentions: &mut Vec<String>, text: String) -> String {
    if mentions.is_empty() {
        return text;
    }
    let m = mentions
        .drain(..)
        .map(|p| format!("@{p}"))
        .collect::<Vec<_>>()
        .join(" ");
    format!("{m}\n{text}")
}

/// Handle a [`remote::RemoteInput::Attach`] (the delivery leg of `POST /api/upload`): an image
/// becomes vision input on the session's next turn; a text file a pending `@path` mention.
///
/// The path is confined to the session's `.forge/uploads/` scratch area (canonicalized — no
/// symlink or `..` escape): `Attach` exists only to deliver uploads, so a WS client injecting
/// an arbitrary host path (`~/.ssh/id_rsa`) is refused with a note instead of read.
/// Confine an upload path to `<cwd>/.forge/uploads/` (canonicalized — no symlink/`..` escape).
/// Shared by [`handle_remote_attach`] (the ambient `Attach` input) and
/// [`resolve_prompt_attachments`] (the explicit, message-correlated attachment list on a
/// `Prompt`) — both exist only to deliver `POST /api/upload` results, so an arbitrary host path
/// (e.g. a WS client probing for secret files) must be refused either way.
fn remote_attach_confined(path: &str, cwd: &str) -> bool {
    let root = std::path::Path::new(cwd).join(".forge").join("uploads");
    std::fs::canonicalize(path)
        .ok()
        .zip(std::fs::canonicalize(&root).ok())
        .map(|(p, r)| p.starts_with(&r))
        .unwrap_or(false)
}

pub(crate) async fn handle_remote_attach(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    mentions: &mut Vec<String>,
    cwd: &str,
    path: String,
    image: bool,
) {
    if !remote_attach_confined(&path, cwd) {
        app.note("⚠ attach ignored — not a file from this session's upload area");
        return;
    }
    if image {
        match crate::image_input::load_image_file(&path) {
            Ok((att, label)) => {
                session.lock().await.attach_images(vec![att]);
                // Also record a `@path` mention, exactly like the non-image branch below: the
                // vision attachment only rides THIS turn's provider call (`attach_images` is
                // transient), so without a durable mention the image reference never reaches
                // persisted history — it renders fine live, then silently vanishes after any
                // history reload (new device, app restart). The mention gives the mobile client
                // something resolvable to detect and re-render on reload via `GET /api/upload`.
                mentions.push(path);
                app.note(&format!(
                    "🖼 image attached ({label}) — rides the next prompt"
                ));
            }
            Err(e) => app.note(&format!("⚠ image attach failed: {e}")),
        }
    } else {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());
        mentions.push(path);
        app.note(&format!(
            "📎 attached {name} — included with the next prompt"
        ));
    }
}

/// Resolve a [`remote::RemoteInput::Prompt`]'s explicit, message-correlated `attachments` list
/// (mobile-upload-race fix): when non-empty it is AUTHORITATIVE for this turn, so any stale
/// `pending_images` left over from an unrelated `Attach` (e.g. an image already uploading for a
/// different, adjacent message) is discarded first — it must never leak into this turn — then
/// each listed attachment is resolved fresh with the same confinement check
/// [`handle_remote_attach`] uses. Images ride straight onto the session; non-image files come
/// back as plain paths (the caller prepends them onto the prompt text as `@path` mentions via
/// [`prepend_attach_mentions`] itself, at the point where the old ambient mentions were applied —
/// this function never touches `text`, so a `//`-escape or `/command` dispatched off the SAME
/// prompt still parses cleanly).
///
/// An empty list (older client, or a plain message with genuinely no attachments) is a no-op that
/// returns an empty `Vec` without touching any session state — callers fall back to exactly the
/// pre-existing ambient `Attach`-then-`Prompt` behavior.
pub(crate) async fn resolve_prompt_attachments(
    session: &Arc<tokio::sync::Mutex<Session>>,
    app: &mut forge_tui::App,
    remote_notes: &mut Vec<String>,
    cwd: &str,
    attachments: Vec<remote::PromptAttachment>,
) -> Vec<String> {
    if attachments.is_empty() {
        return Vec::new();
    }
    // Drop, don't use: whatever's ambiently pending belongs to no turn now that an explicit,
    // authoritative list has arrived for THIS one.
    let _ = session.lock().await.take_pending_images();

    let mut mentions = Vec::new();
    for att in attachments {
        if !remote_attach_confined(&att.path, cwd) {
            tracing::warn!(
                path = %att.path,
                cwd = %cwd,
                "prompt attachment rejected: outside session's upload area"
            );
            push_remote_note(
                remote_notes,
                "⚠ attach ignored — not a file from this session's upload area",
            );
            continue;
        }
        if att.image {
            match crate::image_input::load_image_file(&att.path) {
                Ok((img, label)) => {
                    tracing::info!(path = %att.path, %label, "prompt image attachment resolved");
                    session.lock().await.attach_images(vec![img]);
                    app.note(&format!("🖼 image attached ({label}) — rides this prompt"));
                }
                Err(e) => {
                    tracing::warn!(path = %att.path, error = %e, "prompt image attachment failed to load");
                    app.note(&format!("⚠ image attach failed: {e}"));
                }
            }
        } else {
            mentions.push(att.path);
        }
    }
    mentions
}

/// Start or stop remote control in response to `/remote`. On: bind the server (LAN-reachable by
/// default, loopback with `--local`, or piped through a public tunnel with `--anywhere`), print
/// the connect URL + a scan-to-connect QR code into scrollback, and light the statusline
/// indicator. Off: drop the handle (stops the server + tunnel, frees the port) and clear the
/// indicator. Idempotent: `/remote` toggles, so running it again turns it off.
///
/// `host_override` (`[remote] host`) replaces the auto-discovered LAN IP in the connect
/// URL/QR/cert; only meaningful for the LAN exposure.
pub(crate) async fn toggle_remote(
    remote: &mut Option<remote::RemoteControl>,
    app: &mut forge_tui::App,
    _tui: &mut forge_tui::Tui,
    exposure: remote::Exposure,
    remote_cfg: &forge_config::RemoteConfig,
    history: remote::HistoryProvider,
    workspace: &std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
) -> Result<()> {
    if let Some(rc) = remote.take() {
        // Turning it off: the handle's Drop aborts the server task + tunnel and sends a `closed`
        // snapshot so any connected browser stops reconnecting.
        app.remote_active = false;
        app.note("◉ remote control off — browser disconnected");
        drop(rc);
        return Ok(());
    }
    let anywhere = exposure == remote::Exposure::Anywhere;
    if anywhere {
        app.note("◉ remote control — opening a public tunnel (this can take a few seconds)…");
    }
    let workspace = workspace.clone();
    let started = match exposure {
        remote::Exposure::Anywhere => {
            remote::start_anywhere(Some(history), remote_cfg, Some(&workspace)).await
        }
        other => remote::start(
            other,
            remote_cfg.host.as_deref(),
            Some(history),
            Some(&workspace),
        ),
    };
    match started {
        Ok(rc) => {
            app.remote_active = true;
            let where_ = match exposure {
                remote::Exposure::Lan => "LAN".to_string(),
                remote::Exposure::Local => "loopback".to_string(),
                remote::Exposure::Anywhere => {
                    format!("public tunnel via {}", rc.tunnel.unwrap_or("tunnel"))
                }
            };
            app.note(&format!(
                "◉ remote control on — listening on {} ({where_})",
                rc.url.addr,
            ));
            if anywhere {
                // A public URL is reachable from the whole internet; the path token is the only
                // gate. Make that explicit so the user knows what they've opened.
                app.note(
                    "  ⚠ anyone with the link can drive this session — the token is the only gate",
                );
            }
            app.note(&format!("  connect: {}", rc.url.url));
            if let Some(qr) = remote::qr_lines(&rc.url.url) {
                app.print_lines(qr);
            }
            *remote = Some(rc);
        }
        Err(e) => {
            app.note(&format!("⚠ could not start remote control: {e}"));
        }
    }
    Ok(())
}
