//! `/status`: open the inspector that lists every fact the header and status line can drop.

pub(super) fn run(app: &mut forge_tui::App) {
    app.status_open = true;
}
