//! End-to-end checks against a real device.
//!
//! Skipped unless `FORGE_DEVICE_LIVE=1` and a device is connected, because CI has no phone. Run
//! them against an emulator with:
//!
//! ```text
//! emulator -avd <name> -no-window &
//! FORGE_DEVICE_LIVE=1 cargo test -p forge-agent-device --test live_device -- --test-threads=1
//! ```
//!
//! These exist because every interesting failure in this crate is a failure of adb's real output
//! shape — a dump that comes back empty, a `wm size` line that has an override, an `input text`
//! escape the shell eats — and none of that is reachable from a unit test.

use std::time::Duration;

use forge_device::{ui::Selector, Device};

/// The device under test, booting a light emulator if nothing is connected.
///
/// Booting on demand rather than requiring one up front keeps the suite order-independent: the
/// shutdown test below deliberately leaves no device behind, and every other test has to cope.
async fn device() -> Option<Device> {
    if std::env::var("FORGE_DEVICE_LIVE").ok().as_deref() != Some("1") {
        return None;
    }
    if let Ok(device) = forge_device::connect(None).await {
        return Some(device);
    }
    let adb = forge_device::Adb::locate().expect("adb must be installed to run the live suite");
    let avds = forge_device::emulator::list_avds()
        .await
        .expect("list AVDs");
    let avd = avds
        .first()
        .expect("the live suite needs at least one AVD")
        .clone();
    let options = forge_device::BootOptions {
        headless: true,
        ..forge_device::BootOptions::light()
    };
    let serial = forge_device::emulator::start(&adb, &avd, &options, Duration::from_secs(420))
        .await
        .unwrap_or_else(|error| panic!("could not boot {avd}: {error}"));
    Some(Device::new(adb.with_serial(&serial)))
}

#[tokio::test]
async fn reports_a_plausible_profile() {
    let Some(device) = device().await else { return };
    let profile = device.profile().await.expect("read the device profile");
    assert!(
        !profile.android_version.is_empty(),
        "no android version: {profile:?}"
    );
    assert!(
        profile.sdk.parse::<u32>().is_ok(),
        "sdk is not a number: {profile:?}"
    );
    let (width, height) = profile.screen.expect("screen size");
    assert!(
        width > 0 && height > 0,
        "implausible screen {width}x{height}"
    );
}

#[tokio::test]
async fn dumps_a_hierarchy_with_tappable_nodes() {
    let Some(device) = device().await else { return };
    device.key("home").await.expect("go home");
    let tree = device.dump_ui().await.expect("dump the screen");
    assert!(!tree.nodes.is_empty());
    let (interesting, _) = tree.interesting();
    assert!(
        !interesting.is_empty(),
        "the launcher should expose actionable nodes"
    );
    assert!(
        interesting.iter().all(|node| !node.bounds.is_empty()),
        "a compact dump must not include zero-area nodes"
    );
}

#[tokio::test]
async fn screenshots_are_real_pngs() {
    let Some(device) = device().await else { return };
    let bytes = device.screenshot().await.expect("screencap");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG header");
    assert!(
        bytes.len() > 1024,
        "suspiciously small screenshot: {} bytes",
        bytes.len()
    );
}

#[tokio::test]
async fn settings_round_trip_through_the_settings_app() {
    let Some(device) = device().await else { return };
    // Drives the same path a test flow uses: launch by package, wait for the screen, read it.
    device.force_stop("com.android.settings").await.ok();
    device
        .launch("com.android.settings", None)
        .await
        .expect("launch settings");
    let want = Selector {
        class: Some("TextView".into()),
        ..Default::default()
    };
    let (node, took) = device
        .wait_for(&want, Duration::from_secs(30))
        .await
        .expect("settings should show text within 30s");
    assert!(took < Duration::from_secs(30));
    assert!(!node.bounds.is_empty());
    let current = device.current_activity().await.expect("current activity");
    assert!(
        current.contains("settings"),
        "settings did not come to the foreground: {current}"
    );
}

#[tokio::test]
async fn logcat_filters_by_level() {
    let Some(device) = device().await else { return };
    let all = forge_device::logcat::read(
        device.adb(),
        &forge_device::Filter {
            limit: 500,
            ..Default::default()
        },
    )
    .await
    .expect("read logcat");
    let errors = forge_device::logcat::read(
        device.adb(),
        &forge_device::Filter {
            min_level: Some(forge_device::Level::Error),
            limit: 500,
            ..Default::default()
        },
    )
    .await
    .expect("read logcat at error level");
    assert!(!all.is_empty(), "a booted device always has log lines");
    assert!(errors.len() <= all.len());
    assert!(errors
        .iter()
        .all(|line| line.level >= Some(forge_device::Level::Error)));
}

#[tokio::test]
async fn typing_survives_the_shell() {
    let Some(device) = device().await else { return };
    // `input text` with spaces and metacharacters is the escape path most likely to be wrong, and
    // it fails silently, so assert on what actually landed in the field.
    device.force_stop("com.android.settings").await.ok();
    device
        .launch("com.android.settings", None)
        .await
        .expect("launch settings");
    let search = Selector {
        class: Some("EditText".into()),
        ..Default::default()
    };
    if device
        .wait_for(&search, Duration::from_secs(20))
        .await
        .is_err()
    {
        // Some launchers put search behind a tap; not worth failing the suite over.
        return;
    }
    let node = device
        .tap_selector(&search)
        .await
        .expect("focus the search field");
    assert!(!node.bounds.is_empty());
    device.type_text("wi-fi & data").await.expect("type");
    tokio::time::sleep(Duration::from_millis(800)).await;
    let tree = device.dump_ui().await.expect("re-dump");
    let typed = tree
        .nodes
        .iter()
        .any(|node| node.text.contains("wi-fi & data"));
    assert!(typed, "the escaped text did not reach the field");
}

/// Booting is slow enough to deserve its own gate: `FORGE_DEVICE_LIVE_BOOT=1`.
#[tokio::test]
async fn boots_an_avd_with_the_light_profile_and_shuts_it_down() {
    if std::env::var("FORGE_DEVICE_LIVE_BOOT").ok().as_deref() != Some("1") {
        return;
    }
    let adb = forge_device::Adb::locate().expect("adb");
    let avds = forge_device::emulator::list_avds()
        .await
        .expect("list avds");
    let avd = avds.first().expect("at least one AVD").clone();

    let options = forge_device::BootOptions {
        headless: true,
        ..forge_device::BootOptions::light()
    };
    let serial = forge_device::emulator::start(&adb, &avd, &options, Duration::from_secs(300))
        .await
        .expect("boot the AVD");

    let device = Device::new(adb.with_serial(&serial));
    let profile = device.profile().await.expect("profile the booted device");
    assert!(!profile.android_version.is_empty());
    // The cap actually reached the guest: /proc/meminfo reports a little under what was asked for
    // (the kernel reserves some), so check the order of magnitude rather than an exact figure.
    let meminfo = device
        .adb()
        .shell("cat /proc/meminfo | head -1")
        .await
        .expect("meminfo");
    let total_kb: u64 = meminfo
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .expect("MemTotal");
    assert!(
        (1_400_000..2_200_000).contains(&total_kb),
        "-memory 2048 did not take: MemTotal is {total_kb} kB"
    );

    let took = forge_device::emulator::stop(device.adb(), Duration::from_secs(90))
        .await
        .expect("shut down");
    assert!(took < Duration::from_secs(90));
    let still_listed = adb
        .devices()
        .await
        .expect("list")
        .iter()
        .any(|d| d.serial == serial);
    assert!(
        !still_listed,
        "stop() returned while {serial} was still connected"
    );
}
