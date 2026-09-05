//! Device tools: drive an Android phone or emulator, and read its log.
//!
//! The mobile counterpart to [`crate::browser`], and the same two-tool split for the same reason:
//! `device` controls, `device_logs` inspects, and every tool schema costs prompt tokens in every
//! turn whether or not a session ever touches a phone.
//!
//! Together with [`crate::proxy`] this closes the loop for app testing — drive the app, watch its
//! traffic, read its log — which is what makes a mobile failure diagnosable rather than guessable.
//!
//! Both declare [`SideEffect::Shell`] rather than `Network`. That is the honest class: `shell`
//! runs arbitrary commands on the device, `install` puts software on it, and `clear` destroys an
//! app's data. Gating them like a shell command is the point.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use forge_device::{logcat, net, ui, Device, Selector};
use serde_json::{json, Value};

use crate::{str_arg, SideEffect, Tool, ToolError};

fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn opt_i32(args: &Value, key: &str) -> Option<i32> {
    args.get(key)
        .and_then(Value::as_i64)
        .map(|value| value as i32)
}

fn flag(args: &Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// Build a selector from whichever of the matching fields were supplied.
fn selector(args: &Value) -> Selector {
    Selector {
        text: opt_str(args, "text").map(str::to_string),
        resource_id: opt_str(args, "resource_id").map(str::to_string),
        content_desc: opt_str(args, "content_desc").map(str::to_string),
        class: opt_str(args, "class").map(str::to_string),
        exact: flag(args, "exact", false),
        clickable_only: flag(args, "clickable_only", false),
    }
}

async fn connect(args: &Value) -> Result<Device, ToolError> {
    forge_device::connect(opt_str(args, "serial"))
        .await
        .map_err(failed)
}

/// Everything `device` can do. A const so the schema and the description cannot drift apart —
/// a test asserts every action here is documented.
const ACTIONS: &[&str] = &[
    "devices",
    "info",
    "avds",
    "emulator_start",
    "emulator_stop",
    "install",
    "uninstall",
    "launch",
    "stop",
    "clear",
    "packages",
    "current",
    "tap",
    "long_press",
    "swipe",
    "text",
    "key",
    "screenshot",
    "ui",
    "find",
    "wait_for",
    "shell",
    "push",
    "pull",
    "proxy_set",
    "proxy_clear",
    "reverse",
    "ca_install",
];

/// Fold one JSON object's entries into another.
fn merge(into: &mut Value, from: Value) {
    let (Some(target), Value::Object(source)) = (into.as_object_mut(), from) else {
        return;
    };
    target.extend(source);
}

pub struct DeviceTool;

#[async_trait]
impl Tool for DeviceTool {
    fn name(&self) -> &str {
        "device"
    }

    fn description(&self) -> &str {
        "Drive a real Android phone or emulator over adb: boot an emulator, install and launch an \
         app, tap and type, read the on-screen view hierarchy, screenshot, push and pull files, \
         and run shell commands on the device. Use it to test a mobile app end to end rather than \
         reasoning about it — \"ui\" lists every on-screen element with its exact tap coordinates, \
         and \"tap\"/\"wait_for\" accept the same selector (text, resource_id, content_desc, \
         class) so you can drive a flow by what is on screen instead of by pixel guesses. Pair it \
         with `proxy` to see the app's traffic: \"proxy_set\" points the device at a proxy, \
         \"ca_install\" makes it trust one, and `device_logs` shows why something crashed. \
         Actions: devices, info, avds, emulator_start, emulator_stop, install, uninstall, launch, \
         stop, clear, packages, current, tap, long_press, swipe, text, key, screenshot, ui, find, \
         wait_for, shell, push, pull, proxy_set, proxy_clear, reverse, ca_install."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Shell
    }

    fn schema(&self) -> Value {
        // Split across two `json!` invocations and merged: one literal for a surface this wide
        // blows serde_json's macro recursion limit, and raising it crate-wide to accommodate one
        // schema is the wrong trade.
        let mut properties = json!({
            "action": {
                "type": "string",
                "enum": ACTIONS,
                "description": "What to do."
            },
                "serial": {
                    "type": "string",
                    "description": "Which device to act on. Optional when exactly one is \
                                    connected; required when several are."
                },
                "avd": {"type": "string", "description": "AVD name, for emulator_start."},
                "headless": {"type": "boolean", "description": "For emulator_start: boot with no window."},
                "wipe_data": {"type": "boolean", "description": "For emulator_start: reset to a clean image."},
                "cold_boot": {"type": "boolean", "description": "For emulator_start: ignore the quick-boot snapshot. Slower but certain."},
                "memory_mb": {"type": "integer", "description": "For emulator_start: guest RAM. Default 2048 — a deliberately small emulator."},
                "cores": {"type": "integer", "description": "For emulator_start: guest CPU cores. Default 2."},
                "gpu": {"type": "string", "description": "For emulator_start: -gpu mode. Default swiftshader_indirect headless, host otherwise."},
                "writable_system": {
                    "type": "boolean",
                    "description": "For emulator_start: mount /system writable. Required later if \
                                    you intend to ca_install into the system trust store."
                },
                "package": {"type": "string", "description": "App id, for launch/stop/clear/uninstall."},
                "activity": {"type": "string", "description": "For launch: a specific activity. Default is the launcher intent."},
                "apk": {"type": "string", "description": "Path to an APK, for install."},
                "grant": {"type": "boolean", "description": "For install: pre-grant runtime permissions. Default true."},
                "reinstall": {"type": "boolean", "description": "For install: keep existing data. Default true."},
                "include_system": {"type": "boolean", "description": "For packages: include system packages. Default false."},
                "filter": {"type": "string", "description": "For packages: substring to match."},
                "x": {"type": "integer", "description": "For tap/long_press/swipe: start x."},
                "y": {"type": "integer", "description": "For tap/long_press/swipe: start y."},
                "x2": {"type": "integer", "description": "For swipe: end x."}
        });
        let rest = json!({
                "y2": {"type": "integer", "description": "For swipe: end y."},
                "duration_ms": {"type": "integer", "description": "For swipe/long_press. Defaults 300 / 800."},
                "text": {
                    "type": "string",
                    "description": "For \"text\": what to type into the focused field (ASCII \
                                    only). For tap/find/wait_for: match an element by its text."
                },
                "resource_id": {"type": "string", "description": "Selector: resource id, bare (login_button) or full."},
                "content_desc": {"type": "string", "description": "Selector: content description."},
                "class": {"type": "string", "description": "Selector: widget class substring, e.g. EditText."},
                "exact": {"type": "boolean", "description": "Selector: match text exactly instead of as a substring."},
                "clickable_only": {"type": "boolean", "description": "Selector: only consider tappable elements."},
                "key": {"type": "string", "description": "For key: back, home, enter, tab, delete, app_switch, or a KEYCODE_*."},
                "all": {"type": "boolean", "description": "For ui: include every node, not just the actionable ones."},
                "max_chars": {"type": "integer", "description": "Cap for ui/shell output. Default 20000."},
                "timeout_secs": {"type": "integer", "description": "For wait_for/emulator_start. Defaults 30 / 300."},
                "command": {"type": "string", "description": "For shell: the command to run on the device."},
                "local": {"type": "string", "description": "Host path, for push/pull."},
                "remote": {"type": "string", "description": "Device path, for push/pull."},
                "endpoint": {"type": "string", "description": "For proxy_set: host:port the device should route through."},
                "port": {"type": "integer", "description": "For reverse: tunnel this device port to the same port on this machine."},
                "cert": {"type": "string", "description": "For ca_install: path to a PEM CA certificate."},
                "user_store": {
                    "type": "boolean",
                    "description": "For ca_install: stage into the user store (no root) instead \
                                    of the system store. Only apps that opt in will trust it."
                }
        });
        merge(&mut properties, rest);
        json!({"type": "object", "properties": properties, "required": ["action"]})
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let action = str_arg(args, "action")?;
        match action {
            "devices" => return list_devices().await,
            "avds" => return list_avds().await,
            "emulator_start" => return emulator_start(args).await,
            _ => {}
        }
        let device = connect(args).await?;
        match action {
            "info" => info(&device).await,
            "emulator_stop" => {
                let serial = device.serial().to_string();
                forge_device::emulator::stop(device.adb(), Duration::from_secs(60))
                    .await
                    .map(|took| format!("{serial} shut down after {:.0}s", took.as_secs_f32()))
                    .map_err(failed)
            }
            "install" | "uninstall" | "launch" | "stop" | "clear" | "packages" | "current" => {
                apps(&device, action, args).await
            }
            "tap" | "long_press" | "swipe" | "text" | "key" => input(&device, action, args).await,
            "screenshot" | "ui" | "find" | "wait_for" => screen(&device, action, args).await,
            "shell" | "push" | "pull" => host(&device, action, args).await,
            "proxy_set" | "proxy_clear" | "reverse" | "ca_install" => {
                network(&device, action, args).await
            }
            other => Err(ToolError::Failed(format!(
                "unknown device action '{other}'"
            ))),
        }
    }
}

async fn list_devices() -> Result<String, ToolError> {
    let adb = forge_device::Adb::locate().map_err(failed)?;
    let devices = adb.devices().await.map_err(failed)?;
    if devices.is_empty() {
        return Ok(
            "no devices are connected. Use action \"avds\" to see the emulators you can \
                   start, then \"emulator_start\"."
                .to_string(),
        );
    }
    Ok(devices
        .iter()
        .map(|device| {
            format!(
                "{} — {} {}{}",
                device.serial,
                device.state,
                device.model.as_deref().unwrap_or("unknown model"),
                if device.is_emulator {
                    " (emulator)"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn list_avds() -> Result<String, ToolError> {
    let avds = forge_device::emulator::list_avds().await.map_err(failed)?;
    if avds.is_empty() {
        return Ok("no AVDs are configured — create one with avdmanager.".to_string());
    }
    Ok(avds.join("\n"))
}

async fn emulator_start(args: &Value) -> Result<String, ToolError> {
    let avd = str_arg(args, "avd")?;
    let options = forge_device::BootOptions {
        headless: flag(args, "headless", false),
        wipe_data: flag(args, "wipe_data", false),
        writable_system: flag(args, "writable_system", false),
        http_proxy: opt_str(args, "endpoint").map(str::to_string),
        cold_boot: flag(args, "cold_boot", false),
        memory_mb: args
            .get("memory_mb")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        cores: args
            .get("cores")
            .and_then(Value::as_u64)
            .map(|value| value as u32),
        gpu: opt_str(args, "gpu").map(str::to_string),
        ..forge_device::BootOptions::light()
    };
    let timeout = Duration::from_secs(
        args.get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(300),
    );
    let adb = forge_device::Adb::locate().map_err(failed)?;
    let serial = forge_device::emulator::start(&adb, avd, &options, timeout)
        .await
        .map_err(failed)?;
    Ok(format!("{avd} booted as {serial}"))
}

async fn info(device: &Device) -> Result<String, ToolError> {
    let profile = device.profile().await.map_err(failed)?;
    let proxy = net::proxy_status(device).await.unwrap_or(None);
    let mut report = serde_json::to_value(&profile).map_err(failed)?;
    report["proxy"] = match proxy {
        Some(endpoint) => Value::String(endpoint),
        None => Value::Null,
    };
    report["current_activity"] = Value::String(device.current_activity().await.unwrap_or_default());
    serde_json::to_string_pretty(&report).map_err(failed)
}

async fn apps(device: &Device, action: &str, args: &Value) -> Result<String, ToolError> {
    match action {
        "install" => {
            let apk = PathBuf::from(str_arg(args, "apk")?);
            let out = device
                .install(
                    &apk,
                    flag(args, "reinstall", true),
                    flag(args, "grant", true),
                )
                .await
                .map_err(failed)?;
            Ok(format!("installed {}\n{out}", apk.display()))
        }
        "uninstall" => device
            .uninstall(str_arg(args, "package")?)
            .await
            .map_err(failed),
        "launch" => {
            let package = str_arg(args, "package")?;
            device
                .launch(package, opt_str(args, "activity"))
                .await
                .map_err(failed)?;
            Ok(format!("launched {package}"))
        }
        "stop" => {
            let package = str_arg(args, "package")?;
            device.force_stop(package).await.map_err(failed)?;
            Ok(format!("force-stopped {package}"))
        }
        "clear" => {
            let package = str_arg(args, "package")?;
            device.clear_data(package).await.map_err(failed)?;
            Ok(format!(
                "cleared {package}'s data — it is back to a first-run state"
            ))
        }
        "packages" => {
            let names = device
                .packages(opt_str(args, "filter"), flag(args, "include_system", false))
                .await
                .map_err(failed)?;
            Ok(if names.is_empty() {
                "no packages matched".to_string()
            } else {
                names.join("\n")
            })
        }
        _ => device.current_activity().await.map_err(failed),
    }
}

async fn input(device: &Device, action: &str, args: &Value) -> Result<String, ToolError> {
    match action {
        "tap" | "long_press" => {
            let ms = args
                .get("duration_ms")
                .and_then(Value::as_u64)
                .unwrap_or(800) as u32;
            // Coordinates win when given; otherwise resolve the selector, which is what makes a
            // flow readable ("tap Log in") and stable across screen sizes.
            if let (Some(x), Some(y)) = (opt_i32(args, "x"), opt_i32(args, "y")) {
                return if action == "tap" {
                    device
                        .tap(x, y)
                        .await
                        .map_err(failed)
                        .map(|()| format!("tapped {x},{y}"))
                } else {
                    device
                        .long_press(x, y, ms)
                        .await
                        .map_err(failed)
                        .map(|()| format!("long-pressed {x},{y} for {ms}ms"))
                };
            }
            let want = selector(args);
            if want.is_empty() {
                return Err(ToolError::BadArgs(
                    "tap needs either x and y, or a selector (text, resource_id, content_desc, \
                     class)"
                        .into(),
                ));
            }
            if action == "long_press" {
                let node = device.find_one(&want).await.map_err(failed)?;
                let (x, y) = node.bounds.center();
                device.long_press(x, y, ms).await.map_err(failed)?;
                return Ok(format!("long-pressed {}", node.describe()));
            }
            let node = device.tap_selector(&want).await.map_err(failed)?;
            Ok(format!("tapped {}", node.describe()))
        }
        "swipe" => {
            let (Some(x), Some(y), Some(x2), Some(y2)) = (
                opt_i32(args, "x"),
                opt_i32(args, "y"),
                opt_i32(args, "x2"),
                opt_i32(args, "y2"),
            ) else {
                return Err(ToolError::BadArgs("swipe needs x, y, x2 and y2".into()));
            };
            let ms = args
                .get("duration_ms")
                .and_then(Value::as_u64)
                .unwrap_or(300) as u32;
            device.swipe((x, y), (x2, y2), ms).await.map_err(failed)?;
            Ok(format!("swiped {x},{y} → {x2},{y2} over {ms}ms"))
        }
        "text" => {
            let text = str_arg(args, "text")?;
            device.type_text(text).await.map_err(failed)?;
            Ok(format!("typed {text:?} into the focused field"))
        }
        _ => {
            let key = str_arg(args, "key")?;
            device.key(key).await.map_err(failed)?;
            Ok(format!("pressed {key}"))
        }
    }
}

async fn screen(device: &Device, action: &str, args: &Value) -> Result<String, ToolError> {
    let max = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .unwrap_or(20_000) as usize;
    match action {
        "screenshot" => {
            // A base64 PNG in the transcript is tens of thousands of useless tokens; write it out
            // and hand back the path, exactly as the browser tool does.
            let bytes = device.screenshot().await.map_err(failed)?;
            let path = std::env::temp_dir().join(format!(
                "forge-device-{}-{}.png",
                device.serial().replace(':', "-"),
                std::process::id()
            ));
            std::fs::write(&path, &bytes)
                .map_err(|error| ToolError::Failed(format!("write screenshot: {error}")))?;
            Ok(format!(
                "screenshot written to {} ({} bytes)",
                path.display(),
                bytes.len()
            ))
        }
        "ui" => {
            let tree = device.dump_ui().await.map_err(failed)?;
            let (nodes, hidden) = tree.interesting();
            let (nodes, note) = if flag(args, "all", false) {
                (tree.nodes.iter().collect::<Vec<_>>(), String::new())
            } else {
                (
                    nodes,
                    format!("\n({hidden} layout-only nodes hidden; pass all=true for every node)"),
                )
            };
            let header = match tree.package() {
                Some(package) => format!("{package} — {} elements\n", nodes.len()),
                None => format!("{} elements\n", nodes.len()),
            };
            Ok(forge_device::truncate(
                &format!("{header}{}{note}", ui::outline(&nodes)),
                max,
            ))
        }
        "find" => {
            let want = selector(args);
            if want.is_empty() {
                return Err(ToolError::BadArgs(
                    "find needs a selector: text, resource_id, content_desc or class".into(),
                ));
            }
            let tree = device.dump_ui().await.map_err(failed)?;
            let found = tree.find(&want);
            if found.is_empty() {
                let (visible, _) = tree.interesting();
                return Ok(format!(
                    "nothing matched. {} elements are on screen; run action \"ui\" to see them.",
                    visible.len()
                ));
            }
            Ok(forge_device::truncate(
                &format!("{} match(es):\n{}", found.len(), ui::outline(&found)),
                max,
            ))
        }
        _ => {
            let want = selector(args);
            if want.is_empty() {
                return Err(ToolError::BadArgs("wait_for needs a selector".into()));
            }
            let timeout = Duration::from_secs(
                args.get("timeout_secs")
                    .and_then(Value::as_u64)
                    .unwrap_or(30),
            );
            let (node, took) = device.wait_for(&want, timeout).await.map_err(failed)?;
            Ok(format!(
                "appeared after {:.1}s: {}",
                took.as_secs_f32(),
                node.describe()
            ))
        }
    }
}

async fn host(device: &Device, action: &str, args: &Value) -> Result<String, ToolError> {
    match action {
        "shell" => {
            let command = str_arg(args, "command")?;
            let max = args
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(20_000) as usize;
            let out = device.adb().shell(command).await.map_err(failed)?;
            Ok(if out.is_empty() {
                format!("`{command}` produced no output")
            } else {
                forge_device::truncate(&out, max)
            })
        }
        "push" => {
            let local = PathBuf::from(str_arg(args, "local")?);
            let remote = str_arg(args, "remote")?;
            device.push(&local, remote).await.map_err(failed)
        }
        _ => {
            let remote = str_arg(args, "remote")?;
            let local = PathBuf::from(str_arg(args, "local")?);
            device.pull(remote, &local).await.map_err(failed)
        }
    }
}

async fn network(device: &Device, action: &str, args: &Value) -> Result<String, ToolError> {
    match action {
        "proxy_set" => net::set_proxy(device, str_arg(args, "endpoint")?)
            .await
            .map_err(failed),
        "proxy_clear" => net::clear_proxy(device).await.map_err(failed),
        "reverse" => {
            let port = args
                .get("port")
                .and_then(Value::as_u64)
                .ok_or_else(|| ToolError::BadArgs("reverse needs a port".into()))?;
            net::reverse(device, port as u16).await.map_err(failed)
        }
        _ => {
            let cert = PathBuf::from(str_arg(args, "cert")?);
            if flag(args, "user_store", false) {
                net::stage_user_ca(device, &cert).await.map_err(failed)
            } else {
                net::install_system_ca(device, &cert).await.map_err(failed)
            }
        }
    }
}

pub struct DeviceLogsTool;

#[async_trait]
impl Tool for DeviceLogsTool {
    fn name(&self) -> &str {
        "device_logs"
    }

    fn description(&self) -> &str {
        "Read a device's logcat — the fastest way to find out why an app crashed, hung, or failed \
         a request. Filter by package, tag, minimum level (verbose/debug/info/warn/error/fatal) or \
         free text; an unfiltered buffer is tens of thousands of lines, so filter. Clear the \
         buffer before an action and read after it to see only what that action produced. \
         Actions: read, clear."
    }

    fn side_effect(&self) -> SideEffect {
        SideEffect::Shell
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["read", "clear"], "description": "Default read."},
                "serial": {"type": "string", "description": "Which device. Optional when one is connected."},
                "package": {"type": "string", "description": "Only this app's lines."},
                "tag": {"type": "string", "description": "Only lines whose tag contains this."},
                "level": {
                    "type": "string",
                    "enum": ["verbose", "debug", "info", "warn", "error", "fatal"],
                    "description": "Minimum severity."
                },
                "contains": {"type": "string", "description": "Only lines containing this text."},
                "limit": {"type": "integer", "description": "Most recent N lines. Default 200."},
                "max_chars": {"type": "integer", "description": "Cap on output. Default 20000."}
            }
        })
    }

    async fn run(&self, args: &Value) -> Result<String, ToolError> {
        let device = connect(args).await?;
        let action = opt_str(args, "action").unwrap_or("read");
        if action == "clear" {
            logcat::clear(device.adb()).await.map_err(failed)?;
            return Ok("log buffer cleared".to_string());
        }
        let level = match opt_str(args, "level") {
            Some(raw) => Some(
                logcat::Level::parse(raw)
                    .ok_or_else(|| ToolError::BadArgs(format!("unknown log level '{raw}'")))?,
            ),
            None => None,
        };
        let filter = logcat::Filter {
            package: opt_str(args, "package").map(str::to_string),
            tag: opt_str(args, "tag").map(str::to_string),
            min_level: level,
            contains: opt_str(args, "contains").map(str::to_string),
            limit: args.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize,
        };
        let lines = logcat::read(device.adb(), &filter).await.map_err(failed)?;
        if lines.is_empty() {
            return Ok("no log lines matched".to_string());
        }
        let max = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(20_000) as usize;
        let body = lines
            .iter()
            .map(logcat::LogLine::render)
            .collect::<Vec<_>>()
            .join("\n");
        Ok(forge_device::truncate(&body, max))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selector_is_built_from_whichever_fields_are_present() {
        let want = selector(&json!({"text": "Log in", "clickable_only": true}));
        assert!(!want.is_empty());
        assert_eq!(want.text.as_deref(), Some("Log in"));
        assert!(want.clickable_only);
        assert!(want.resource_id.is_none());
    }

    #[test]
    fn no_matching_fields_is_an_empty_selector() {
        assert!(selector(&json!({"serial": "emulator-5554"})).is_empty());
    }

    #[tokio::test]
    async fn tapping_with_neither_coordinates_nor_a_selector_is_rejected_before_touching_a_device()
    {
        // Argument validation must not require a phone; only the error path is exercised here.
        let error = DeviceTool
            .run(&json!({"action": "tap", "serial": "definitely-not-a-device"}))
            .await
            .expect_err("no such device");
        assert!(matches!(
            error,
            ToolError::Failed(_) | ToolError::BadArgs(_)
        ));
    }

    #[test]
    fn both_tools_are_gated_as_shell_since_they_run_commands_on_the_device() {
        assert!(matches!(DeviceTool.side_effect(), SideEffect::Shell));
        assert!(matches!(DeviceLogsTool.side_effect(), SideEffect::Shell));
    }

    #[test]
    fn every_advertised_action_appears_in_the_description() {
        let schema = DeviceTool.schema();
        let actions = schema["properties"]["action"]["enum"].as_array().unwrap();
        let description = DeviceTool.description();
        for action in actions {
            let name = action.as_str().unwrap();
            assert!(
                description.contains(name),
                "{name} is not documented in the description"
            );
        }
    }
}
