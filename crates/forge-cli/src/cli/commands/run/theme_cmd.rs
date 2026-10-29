//! `/theme [name]` — list the accent palettes, or switch to one live and save it as `tui.palette`.

pub(crate) fn run_theme(app: &mut forge_tui::App, name: Option<String>) {
    let config = forge_config::load().unwrap_or_default().tui;
    let Some(name) = name else {
        let names = forge_tui::theme::palettes::NAMES.join(", ");
        app.note(&format!(
            "◐ accent palette: {} — available: {names}. `/theme <name>` switches and saves it; per-role colours go in [tui.colors].",
            config.palette
        ));
        return;
    };
    match forge_tui::theme::set_palette(&name, &config.colors) {
        Ok(rejected) => {
            for entry in rejected {
                app.note(&format!("◐ ignored [tui.colors] entry: {entry}"));
            }
            match forge_config::set_config_value(
                forge_config::ConfigScope::User,
                "tui.palette",
                &name,
            ) {
                Ok(()) => app.note(&format!("◐ accent palette set to {name} and saved")),
                Err(error) => app.note(&format!(
                    "◐ accent palette set to {name} for this session; could not save it: {error}"
                )),
            }
        }
        Err(message) => app.note(&format!("◐ {message}")),
    }
}
