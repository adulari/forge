//! Pointing a device's traffic at a proxy, and trusting the proxy's certificate.
//!
//! This is what joins this crate to `forge-proxy`: the proxy can only read an app's HTTPS if the
//! device both routes through it and trusts its CA. Android 7 moved user-installed CAs out of the
//! default trust store, so on a modern image only the system store works — which needs root, i.e.
//! an emulator. That constraint is enforced here rather than surfacing as an opaque TLS error.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::process::Command;

use crate::adb::which;
use crate::session::Device;

/// Route the device's HTTP(S) traffic through `host:port`.
pub async fn set_proxy(device: &Device, endpoint: &str) -> Result<String> {
    if !endpoint.contains(':') {
        bail!("proxy endpoint must be host:port, got {endpoint:?}");
    }
    device
        .adb()
        .shell(&format!("settings put global http_proxy {endpoint}"))
        .await?;
    Ok(format!("device traffic now routes through {endpoint}"))
}

/// Stop routing through a proxy.
pub async fn clear_proxy(device: &Device) -> Result<String> {
    // `:0` is the documented "no proxy" value; deleting the setting leaves some builds pointed at
    // a stale endpoint until reboot.
    device
        .adb()
        .shell("settings put global http_proxy :0")
        .await?;
    device
        .adb()
        .shell("settings delete global global_http_proxy_host")
        .await
        .ok();
    device
        .adb()
        .shell("settings delete global global_http_proxy_port")
        .await
        .ok();
    Ok("device proxy cleared".to_string())
}

/// The proxy endpoint currently configured, if any.
pub async fn proxy_status(device: &Device) -> Result<Option<String>> {
    let value = device.adb().shell("settings get global http_proxy").await?;
    let value = value.trim();
    Ok(match value {
        "" | "null" | ":0" => None,
        other => Some(other.to_string()),
    })
}

/// Make the device reach a proxy running on this machine's localhost.
///
/// `adb reverse` opens a tunnel from the device back to the host, which is the only thing that
/// works when the proxy is bound to 127.0.0.1 and not exposed on the LAN.
pub async fn reverse(device: &Device, port: u16) -> Result<String> {
    let spec = format!("tcp:{port}");
    device.adb().run(&["reverse", &spec, &spec]).await?;
    Ok(format!(
        "device port {port} now tunnels to this machine's 127.0.0.1:{port}"
    ))
}

/// Install a PEM CA certificate into the **system** trust store.
///
/// Requires a rooted device or an emulator booted with `-writable-system`; on anything else this
/// reports what is missing instead of half-applying.
pub async fn install_system_ca(device: &Device, pem: &Path) -> Result<String> {
    if !pem.is_file() {
        bail!("no certificate at {}", pem.display());
    }
    let hash = subject_hash_old(pem).await?;
    let adb = device.adb();

    // `adb root` on a production build fails loudly; on an emulator it restarts adbd as root.
    adb.run(&["root"]).await.ok();
    // adbd restarting drops the connection for a moment.
    adb.run(&["wait-for-device"]).await.ok();
    if adb.shell("id -u").await?.trim() != "0" {
        bail!(
            "the device's adb shell is not root, so the system trust store cannot be written. \
             Boot an emulator with a Google APIs (not Play) image and pass writable_system=true."
        );
    }
    adb.run(&["remount"]).await.ok();
    // Android 10+ mounts the cert directory from an APEX; a tmpfs overlay is the way in.
    let target = format!("/system/etc/security/cacerts/{hash}.0");
    let staged = "/data/local/tmp/forge-ca.pem";
    device.push(pem, staged).await?;
    let script = format!(
        "mount -o rw,remount /system 2>/dev/null; \
         cp {staged} {target} && chmod 644 {target} && chown root:root {target} && echo OK"
    );
    let out = adb.shell_timeout(&script, Duration::from_secs(120)).await?;
    if !out.contains("OK") {
        bail!(
            "could not write {target}: {out}\nOn Android 10+ the store is read-only unless the \
             emulator was started with -writable-system."
        );
    }
    Ok(format!(
        "installed the CA as {target}; reboot the device for every app to pick it up"
    ))
}

/// Place a CA in the user store, where the user must still confirm it in Settings.
///
/// Only apps that opt into user CAs will trust it, but it needs no root, so it is the fallback on
/// a physical phone.
pub async fn stage_user_ca(device: &Device, pem: &Path) -> Result<String> {
    if !pem.is_file() {
        bail!("no certificate at {}", pem.display());
    }
    let remote = "/sdcard/Download/forge-ca.crt";
    device.push(pem, remote).await?;
    Ok(format!(
        "copied the CA to {remote}. Install it under Settings → Security → Encryption & \
         credentials → Install a certificate → CA certificate. Note that on Android 7+ only apps \
         that opt in will trust a user CA."
    ))
}

/// OpenSSL's legacy subject hash, which is the filename Android's trust store requires.
async fn subject_hash_old(pem: &Path) -> Result<String> {
    let openssl = which("openssl").context(
        "openssl is needed to compute the certificate's trust-store filename but was not found",
    )?;
    let out = Command::new(openssl)
        .args(["x509", "-inform", "PEM", "-subject_hash_old", "-in"])
        .arg(pem)
        .output()
        .await
        .context("run openssl")?;
    if !out.status.success() {
        bail!(
            "openssl could not read {}: {}",
            pem.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let hash = text.lines().next().unwrap_or_default().trim();
    if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("openssl returned an unexpected subject hash: {text:?}");
    }
    Ok(hash.to_string())
}
