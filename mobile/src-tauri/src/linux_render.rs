//! Chooses the WebKitGTK/GDK environment workarounds Linux needs, before GTK or WebKit start.
//!
//! The decision is a pure function (`decide`: environment map + probed `Facts` -> `Plan`) so every
//! combination is unit-tested; `init` only gathers the inputs, applies the plan and records it.
//!
//! What each workaround is for, and the evidence it rests on:
//! - `WEBKIT_DISABLE_DMABUF_RENDERER=1`: WebKitGTK's DMABUF renderer exits with `Gdk Error 71` on
//!   Hyprland/wlroots, and on Wayland + NVIDIA (explicit sync). Set unconditionally since the first
//!   desktop releases, so it stays the default everywhere; `WEBKIT_DISABLE_DMABUF_RENDERER=0`
//!   brings the renderer back.
//! - `GDK_BACKEND=x11`: WebKitGTK 2.54 paints a black window on native Wayland when the NVIDIA
//!   proprietary driver drives the display; XWayland renders normally. Applied only when all of
//!   Wayland, a proprietary-NVIDIA display GPU, WebKitGTK >= 2.54 and a reachable X display hold,
//!   because XWayland costs fractional scaling and native global shortcuts.
//! - the `drmWaitVBlank` shim (`vblank.rs`) is compiled in and only acts when the driver answers
//!   `EOPNOTSUPP` (NVIDIA); here it can only be switched off.
//!
//! A value the user set is never overwritten. `FORGE_DESKTOP_NO_WEBKIT_WORKAROUNDS=1` turns the
//! whole module off.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;

pub const OPT_OUT: &str = "FORGE_DESKTOP_NO_WEBKIT_WORKAROUNDS";
pub const BACKEND_OVERRIDE: &str = "FORGE_DESKTOP_GDK_BACKEND";
pub const NO_VBLANK_SHIM: &str = "FORGE_DESKTOP_NO_VBLANK_SHIM";

/// First WebKitGTK minor release known to paint a black window on Wayland + NVIDIA.
const BLACK_WINDOW_MINOR: u32 = 54;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gpu {
    pub vendor: Vendor,
    pub driver: Option<String>,
    pub drives_display: bool,
}

/// Everything `decide` needs that is not an environment variable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub gpus: Vec<Gpu>,
    /// (major, minor) of the WebKitGTK this process is linked against.
    pub webkit: Option<(u32, u32)>,
    pub in_appimage: bool,
}

impl Facts {
    fn proprietary_nvidia_display(&self) -> bool {
        self.gpus.iter().any(|gpu| {
            gpu.vendor == Vendor::Nvidia
                && gpu.driver.as_deref() == Some("nvidia")
                && gpu.drives_display
        })
    }

    fn webkit_black_window_prone(&self) -> bool {
        self.webkit
            .is_some_and(|(major, minor)| major > 2 || (major == 2 && minor >= BLACK_WINDOW_MINOR))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub key: &'static str,
    pub value: String,
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub disabled: bool,
    pub session: Session,
    pub compositor: Option<String>,
    pub set: Vec<Change>,
    /// User-provided values that were left alone, as `KEY=value`.
    pub kept: Vec<String>,
    pub vblank_shim: bool,
    pub notes: Vec<String>,
}

pub fn truthy(value: Option<&String>) -> bool {
    value.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn non_empty<'a>(env: &'a BTreeMap<String, String>, key: &str) -> Option<&'a str> {
    env.get(key).map(String::as_str).filter(|v| !v.is_empty())
}

pub fn session(env: &BTreeMap<String, String>) -> Session {
    if non_empty(env, "WAYLAND_DISPLAY").is_some() {
        Session::Wayland
    } else if non_empty(env, "XDG_SESSION_TYPE") == Some("x11")
        || non_empty(env, "DISPLAY").is_some()
    {
        Session::X11
    } else {
        Session::Unknown
    }
}

pub fn compositor(env: &BTreeMap<String, String>) -> Option<String> {
    if non_empty(env, "HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return Some("hyprland".into());
    }
    if non_empty(env, "SWAYSOCK").is_some() {
        return Some("sway".into());
    }
    let desktop =
        non_empty(env, "XDG_CURRENT_DESKTOP").or_else(|| non_empty(env, "XDG_SESSION_DESKTOP"))?;
    let first = desktop.split(':').next().unwrap_or(desktop);
    Some(first.to_ascii_lowercase())
}

pub fn decide(env: &BTreeMap<String, String>, facts: &Facts) -> Plan {
    let session = session(env);
    let compositor = compositor(env);
    let mut plan = Plan {
        disabled: false,
        session,
        compositor,
        set: Vec::new(),
        kept: Vec::new(),
        vblank_shim: true,
        notes: Vec::new(),
    };
    if truthy(env.get(OPT_OUT)) {
        plan.disabled = true;
        plan.vblank_shim = false;
        plan.notes
            .push(format!("{OPT_OUT} set: no workarounds applied"));
        return plan;
    }
    plan.vblank_shim = !truthy(env.get(NO_VBLANK_SHIM));

    let mut backend = non_empty(env, "GDK_BACKEND").map(str::to_owned);
    if let Some(user) = &backend {
        plan.kept.push(format!("GDK_BACKEND={user}"));
    } else {
        match non_empty(env, BACKEND_OVERRIDE)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some(choice @ ("x11" | "wayland")) => {
                plan.set.push(Change {
                    key: "GDK_BACKEND",
                    value: choice.into(),
                    why: format!("{BACKEND_OVERRIDE}={choice}"),
                });
                backend = Some(choice.into());
            }
            other => {
                if let Some(bad) = other.filter(|v| *v != "auto") {
                    plan.notes.push(format!(
                        "{BACKEND_OVERRIDE}={bad} ignored (use auto, x11 or wayland)"
                    ));
                }
                if session == Session::Wayland
                    && facts.proprietary_nvidia_display()
                    && facts.webkit_black_window_prone()
                    && non_empty(env, "DISPLAY").is_some()
                {
                    plan.set.push(Change {
                        key: "GDK_BACKEND",
                        value: "x11".into(),
                        why: "Wayland + proprietary NVIDIA + WebKitGTK >= 2.54 paints a black window natively".into(),
                    });
                    backend = Some("x11".into());
                }
            }
        }
    }

    match non_empty(env, "WEBKIT_DISABLE_DMABUF_RENDERER") {
        Some(user) => plan
            .kept
            .push(format!("WEBKIT_DISABLE_DMABUF_RENDERER={user}")),
        None => {
            let wayland = match backend.as_deref() {
                Some(b) => b == "wayland",
                None => session == Session::Wayland,
            };
            let why = if wayland {
                "the DMABUF renderer exits with Gdk Error 71 on Wayland (wlroots, NVIDIA explicit sync)"
            } else {
                "compatibility default, unchanged from earlier releases"
            };
            plan.set.push(Change {
                key: "WEBKIT_DISABLE_DMABUF_RENDERER",
                value: "1".into(),
                why: why.into(),
            });
        }
    }
    plan
}

fn session_label(session: Session) -> &'static str {
    match session {
        Session::Wayland => "wayland",
        Session::X11 => "x11",
        Session::Unknown => "unknown",
    }
}

fn vendor_label(vendor: Vendor) -> &'static str {
    match vendor {
        Vendor::Nvidia => "nvidia",
        Vendor::Amd => "amd",
        Vendor::Intel => "intel",
        Vendor::Other => "other",
    }
}

/// Multi-line-free summary for the app log.
pub fn log_line(plan: &Plan, facts: &Facts) -> String {
    let gpus: Vec<String> = facts
        .gpus
        .iter()
        .map(|g| {
            format!(
                "{}[{}{}]",
                vendor_label(g.vendor),
                g.driver.as_deref().unwrap_or("?"),
                if g.drives_display { ",display" } else { "" }
            )
        })
        .collect();
    let webkit = facts
        .webkit
        .map_or("?".to_string(), |(a, b)| format!("{a}.{b}"));
    let set: Vec<String> = plan
        .set
        .iter()
        .map(|c| format!("{}={} ({})", c.key, c.value, c.why))
        .collect();
    format!(
        "forge: linux webkit workarounds: session={}{} gpu={} webkitgtk={} appimage={} set=[{}] kept=[{}] vblank_shim={}{}",
        session_label(plan.session),
        plan.compositor.as_deref().map(|c| format!("/{c}")).unwrap_or_default(),
        if gpus.is_empty() { "none".into() } else { gpus.join("+") },
        webkit,
        facts.in_appimage,
        set.join("; "),
        plan.kept.join(", "),
        if plan.vblank_shim { "on" } else { "off" },
        if plan.notes.is_empty() { String::new() } else { format!(" notes=[{}]", plan.notes.join("; ")) },
    )
}

/// One short line for Settings -> Diagnostics and the About panel.
pub fn short_line(plan: &Plan, facts: &Facts) -> String {
    if plan.disabled {
        return format!("WebKit workarounds off ({OPT_OUT})");
    }
    let mut parts = vec![format!(
        "{}{}",
        session_label(plan.session),
        plan.compositor
            .as_deref()
            .map(|c| format!(" ({c})"))
            .unwrap_or_default()
    )];
    if let Some(gpu) = facts
        .gpus
        .iter()
        .find(|g| g.drives_display)
        .or(facts.gpus.first())
    {
        parts.push(vendor_label(gpu.vendor).to_string());
    }
    if let Some((a, b)) = facts.webkit {
        parts.push(format!("WebKitGTK {a}.{b}"));
    }
    let backend = plan
        .set
        .iter()
        .find(|c| c.key == "GDK_BACKEND")
        .map(|c| format!("GDK_BACKEND={} (auto)", c.value));
    let kept = plan
        .kept
        .iter()
        .find(|k| k.starts_with("GDK_BACKEND="))
        .map(|k| format!("{k} (yours)"));
    parts.extend(backend.or(kept));
    let dmabuf = plan
        .set
        .iter()
        .any(|c| c.key == "WEBKIT_DISABLE_DMABUF_RENDERER");
    parts.push(if dmabuf {
        "DMABUF off".into()
    } else {
        "DMABUF renderer per your setting".into()
    });
    parts.push(format!(
        "vblank shim {}",
        if plan.vblank_shim { "on" } else { "off" }
    ));
    parts.join(" · ")
}

pub fn parse_vendor(raw: &str) -> Vendor {
    match raw.trim().to_ascii_lowercase().trim_start_matches("0x") {
        "10de" => Vendor::Nvidia,
        "1002" | "1022" => Vendor::Amd,
        "8086" => Vendor::Intel,
        _ => Vendor::Other,
    }
}

fn probe_gpus(drm: &Path, nvidia_module_loaded: bool) -> Vec<Gpu> {
    let Ok(entries) = std::fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards: Vec<_> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            name.strip_prefix("card")
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    cards.sort_by_key(|e| e.file_name());
    let mut gpus = Vec::new();
    for card in cards {
        let device = card.path().join("device");
        let Ok(vendor) = std::fs::read_to_string(device.join("vendor")) else {
            continue;
        };
        let vendor = parse_vendor(&vendor);
        let mut driver = std::fs::read_link(device.join("driver"))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
        if driver.is_none() && vendor == Vendor::Nvidia && nvidia_module_loaded {
            driver = Some("nvidia".into());
        }
        let prefix = format!("{}-", card.file_name().to_string_lossy());
        let drives_display =
            std::fs::read_dir(drm)
                .into_iter()
                .flatten()
                .flatten()
                .any(|connector| {
                    connector.file_name().to_string_lossy().starts_with(&prefix)
                        && std::fs::read_to_string(connector.path().join("status"))
                            .is_ok_and(|s| s.trim() == "connected")
                });
        gpus.push(Gpu {
            vendor,
            driver,
            drives_display,
        });
    }
    gpus
}

extern "C" {
    fn webkit_get_major_version() -> std::ffi::c_uint;
    fn webkit_get_minor_version() -> std::ffi::c_uint;
}

fn probe() -> Facts {
    let nvidia_module_loaded = Path::new("/sys/module/nvidia").exists()
        || Path::new("/proc/driver/nvidia/version").exists();
    // SAFETY: both are pure getters in libwebkit2gtk, which this binary is linked against; they
    // read compile-time constants and need no GTK initialisation.
    let webkit = Some(unsafe { (webkit_get_major_version(), webkit_get_minor_version()) });
    Facts {
        gpus: probe_gpus(Path::new("/sys/class/drm"), nvidia_module_loaded),
        webkit,
        in_appimage: std::env::var_os("APPDIR").is_some() && std::env::var_os("APPIMAGE").is_some(),
    }
}

static RECORDED: OnceLock<String> = OnceLock::new();

/// The short diagnostics line, once `init` has run.
pub fn diagnostics_line() -> Option<String> {
    RECORDED.get().cloned()
}

/// Call first in `main`, before anything touches GTK or WebKit.
pub fn init() {
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let facts = probe();
    let plan = decide(&env, &facts);
    for change in &plan.set {
        std::env::set_var(change.key, &change.value);
    }
    crate::vblank::set_enabled(plan.vblank_shim);
    crate::appimage_launcher::neutralize_current();
    eprintln!("{}", log_line(&plan, &facts));
    let _ = RECORDED.set(short_line(&plan, &facts));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn gpu(vendor: Vendor, driver: &str, display: bool) -> Gpu {
        Gpu {
            vendor,
            driver: Some(driver.into()),
            drives_display: display,
        }
    }

    fn nvidia(webkit: (u32, u32)) -> Facts {
        Facts {
            gpus: vec![gpu(Vendor::Nvidia, "nvidia", true)],
            webkit: Some(webkit),
            in_appimage: true,
        }
    }

    fn value<'a>(plan: &'a Plan, key: &str) -> Option<&'a str> {
        plan.set
            .iter()
            .find(|c| c.key == key)
            .map(|c| c.value.as_str())
    }

    const WAYLAND: &[(&str, &str)] = &[
        ("WAYLAND_DISPLAY", "wayland-1"),
        ("DISPLAY", ":0"),
        ("HYPRLAND_INSTANCE_SIGNATURE", "x"),
    ];

    #[test]
    fn wayland_nvidia_webkit_254_falls_back_to_xwayland() {
        let plan = decide(&env(WAYLAND), &nvidia((2, 54)));
        assert_eq!(value(&plan, "GDK_BACKEND"), Some("x11"));
        assert_eq!(value(&plan, "WEBKIT_DISABLE_DMABUF_RENDERER"), Some("1"));
        assert_eq!(plan.compositor.as_deref(), Some("hyprland"));
    }

    #[test]
    fn older_webkit_keeps_native_wayland() {
        let plan = decide(&env(WAYLAND), &nvidia((2, 52)));
        assert_eq!(value(&plan, "GDK_BACKEND"), None);
        assert!(plan
            .set
            .iter()
            .any(|c| c.key == "WEBKIT_DISABLE_DMABUF_RENDERER" && c.why.contains("Error 71")));
    }

    #[test]
    fn amd_intel_and_nouveau_keep_native_wayland() {
        for gpus in [
            vec![gpu(Vendor::Amd, "amdgpu", true)],
            vec![gpu(Vendor::Intel, "i915", true)],
            vec![gpu(Vendor::Nvidia, "nouveau", true)],
        ] {
            let facts = Facts {
                gpus,
                webkit: Some((2, 54)),
                in_appimage: false,
            };
            assert_eq!(value(&decide(&env(WAYLAND), &facts), "GDK_BACKEND"), None);
        }
    }

    #[test]
    fn nvidia_that_drives_no_display_does_not_trigger() {
        let facts = Facts {
            gpus: vec![
                gpu(Vendor::Intel, "i915", true),
                gpu(Vendor::Nvidia, "nvidia", false),
            ],
            webkit: Some((2, 54)),
            in_appimage: false,
        };
        assert_eq!(value(&decide(&env(WAYLAND), &facts), "GDK_BACKEND"), None);
    }

    #[test]
    fn no_x_display_means_no_x11_fallback() {
        let plan = decide(&env(&[("WAYLAND_DISPLAY", "wayland-1")]), &nvidia((2, 54)));
        assert_eq!(value(&plan, "GDK_BACKEND"), None);
    }

    #[test]
    fn x11_session_is_never_switched_and_keeps_the_default_renderer_setting() {
        let plan = decide(
            &env(&[("DISPLAY", ":0"), ("XDG_SESSION_TYPE", "x11")]),
            &nvidia((2, 54)),
        );
        assert_eq!(plan.session, Session::X11);
        assert_eq!(value(&plan, "GDK_BACKEND"), None);
        assert!(plan
            .set
            .iter()
            .any(|c| c.key == "WEBKIT_DISABLE_DMABUF_RENDERER" && c.why.contains("compatibility")));
    }

    #[test]
    fn user_values_are_never_overridden() {
        let mut vars = env(WAYLAND);
        vars.insert("GDK_BACKEND".into(), "wayland".into());
        vars.insert("WEBKIT_DISABLE_DMABUF_RENDERER".into(), "0".into());
        let plan = decide(&vars, &nvidia((2, 54)));
        assert!(plan.set.is_empty());
        assert_eq!(
            plan.kept,
            ["GDK_BACKEND=wayland", "WEBKIT_DISABLE_DMABUF_RENDERER=0"]
        );
    }

    #[test]
    fn backend_override_variable_wins_over_detection() {
        let mut vars = env(WAYLAND);
        vars.insert(BACKEND_OVERRIDE.into(), "wayland".into());
        let plan = decide(&vars, &nvidia((2, 54)));
        assert_eq!(value(&plan, "GDK_BACKEND"), Some("wayland"));
        assert!(plan
            .set
            .iter()
            .any(|c| c.key == "WEBKIT_DISABLE_DMABUF_RENDERER" && c.why.contains("Error 71")));

        vars.insert(BACKEND_OVERRIDE.into(), "X11".into());
        assert_eq!(
            value(&decide(&vars, &Facts::default()), "GDK_BACKEND"),
            Some("x11")
        );

        vars.insert(BACKEND_OVERRIDE.into(), "auto".into());
        assert_eq!(
            value(&decide(&vars, &nvidia((2, 54))), "GDK_BACKEND"),
            Some("x11")
        );

        vars.insert(BACKEND_OVERRIDE.into(), "mir".into());
        let plan = decide(&vars, &nvidia((2, 54)));
        assert!(plan.notes.iter().any(|n| n.contains("ignored")));
        assert_eq!(value(&plan, "GDK_BACKEND"), Some("x11"));
    }

    #[test]
    fn master_opt_out_changes_nothing_and_disables_the_shim() {
        let mut vars = env(WAYLAND);
        vars.insert(OPT_OUT.into(), "1".into());
        let plan = decide(&vars, &nvidia((2, 54)));
        assert!(plan.disabled && plan.set.is_empty() && !plan.vblank_shim);
        assert!(short_line(&plan, &nvidia((2, 54))).contains(OPT_OUT));
    }

    #[test]
    fn falsy_opt_out_values_do_not_opt_out() {
        for v in ["0", "", "false", "off"] {
            let mut vars = env(WAYLAND);
            vars.insert(OPT_OUT.into(), v.into());
            assert!(!decide(&vars, &nvidia((2, 54))).disabled, "{v:?}");
        }
    }

    #[test]
    fn shim_has_its_own_switch() {
        let mut vars = env(WAYLAND);
        vars.insert(NO_VBLANK_SHIM.into(), "1".into());
        let plan = decide(&vars, &nvidia((2, 54)));
        assert!(!plan.vblank_shim);
        assert!(!plan.set.is_empty());
    }

    #[test]
    fn compositor_detection() {
        assert_eq!(
            compositor(&env(&[("SWAYSOCK", "/s")])).as_deref(),
            Some("sway")
        );
        assert_eq!(
            compositor(&env(&[("XDG_CURRENT_DESKTOP", "KDE")])).as_deref(),
            Some("kde")
        );
        assert_eq!(
            compositor(&env(&[("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")])).as_deref(),
            Some("ubuntu")
        );
        assert_eq!(compositor(&env(&[])), None);
    }

    #[test]
    fn vendor_ids() {
        assert_eq!(parse_vendor("0x10de\n"), Vendor::Nvidia);
        assert_eq!(parse_vendor("0x1002"), Vendor::Amd);
        assert_eq!(parse_vendor("0x8086"), Vendor::Intel);
        assert_eq!(parse_vendor("0x1234"), Vendor::Other);
    }

    #[test]
    fn log_and_short_lines_name_the_choices() {
        let facts = nvidia((2, 54));
        let plan = decide(&env(WAYLAND), &facts);
        let log = log_line(&plan, &facts);
        assert!(
            log.contains("GDK_BACKEND=x11")
                && log.contains("nvidia[nvidia,display]")
                && log.contains("webkitgtk=2.54")
        );
        assert!(!log.contains('\n'));
        let short = short_line(&plan, &facts);
        assert!(
            short.contains("wayland (hyprland)")
                && short.contains("GDK_BACKEND=x11 (auto)")
                && short.contains("vblank shim on")
        );
    }
}
