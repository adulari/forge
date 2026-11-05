// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let process_start = std::time::Instant::now();
    #[cfg(target_os = "linux")]
    forge_desktop_lib::linux_render::init();
    forge_desktop_lib::run(process_start);
}
