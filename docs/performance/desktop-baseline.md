## Baseline capture status

The harness and export are verified, but this capture intentionally did not launch an interactive Tauri window while the display-target strategy is being decided. Therefore startup, composer, scroll, long-task, and RSS values remain `pending`; no runtime performance claim is made. A release Rust build was started successfully but did not finish within the bounded capture window, so installer size remains pending as well.

The next reproducible capture should launch the retained Tauri binary on the selected display target, open the Diagnostics route, exercise a 10,000-row transcript for at least 30 seconds, type normal and IME-composed text in the composer, and sample the Tauri/WebView process tree with RSS at idle, peak, and after the session. Record each value in this document with the active display's actual refresh rate.
