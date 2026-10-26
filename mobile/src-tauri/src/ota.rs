//! Over-the-air updates for the desktop web UI (docs/features/desktop-ota.md).
//!
//! A signed bundle of the web export is published for every push to main, stamped with the
//! runtime version of the native shell it was built against. A binary only takes bundles whose
//! runtime version equals its own, verifies them against the updater public key, unpacks them
//! beside the app data and serves them in place of the embedded UI on the next launch.
//! Anything that changes the native side changes the runtime version and so needs a full release.

mod assets;
mod bundle;
mod manifest;
mod state;

use assets::{ActiveDir, OtaAssets, Placeholder};
use manifest::{Acceptance, Manifest, Skip};
use serde::Serialize;
use state::{BootNote, BundleRef, State};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_http::reqwest;

const DEFAULT_BASE_URL: &str = "https://github.com/Adulari/forge/releases/download/desktop-ota";
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(5);
const CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
const BOOT_WATCHDOG: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const SIGNATURE_LIMIT: u64 = 8 * 1024;
const PENDING_EVENT: &str = "forge://ota-pending";

#[derive(Clone)]
pub struct Settings {
    pub root: PathBuf,
    pub runtime_version: String,
    pub app_version: String,
    pub pubkey: String,
    pub base_url: String,
    pub embedded_id: String,
    pub embedded_created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    NothingPublished,
    Ignored(Skip),
    Staged(String),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    enabled: bool,
    disabled_reason: Option<String>,
    runtime_version: String,
    app_version: String,
    embedded_bundle_id: String,
    embedded_created_at: u64,
    active: Option<BundleRef>,
    pending: Option<BundleRef>,
    last_check: Option<u64>,
    last_error: Option<String>,
    last_rollback: Option<String>,
    booting: bool,
}

pub struct Ota {
    settings: Settings,
    disabled_reason: Option<String>,
    state: Mutex<State>,
    active_dir: ActiveDir,
    checking: AtomicBool,
    app: OnceLock<AppHandle>,
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Ota {
    fn bundles_dir(&self) -> PathBuf {
        self.settings.root.join("bundles")
    }

    /// Loads the persisted state and decides what this launch serves.
    pub fn new(settings: Settings, disabled_reason: Option<String>) -> Arc<Ota> {
        let ota = Arc::new(Ota {
            state: Mutex::new(State::default()),
            active_dir: Arc::new(RwLock::new(None)),
            checking: AtomicBool::new(false),
            app: OnceLock::new(),
            settings,
            disabled_reason,
        });
        if ota.disabled_reason.is_none() {
            ota.begin_boot();
        }
        ota
    }

    fn begin_boot(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        *state = State::load(&self.settings.root);
        state.retarget(
            &self.settings.runtime_version,
            self.settings.embedded_created_at,
        );
        for note in state.begin_boot() {
            match note {
                BootNote::RolledBack { bad } => {
                    eprintln!("forge: ota bundle {bad} never reported healthy; rolled back")
                }
                BootNote::Promoted { id } => eprintln!("forge: ota serving bundle {id}"),
            }
        }
        // A bundle whose files are gone cannot be served; fall back like any other bad boot.
        while let Some(active) = state.active.clone() {
            if self
                .bundles_dir()
                .join(&active.id)
                .join("index.html")
                .is_file()
            {
                break;
            }
            eprintln!("forge: ota bundle {} is missing on disk", active.id);
            state.abandon_active();
            state.booting = state.active.is_some();
        }
        self.persist(&state);
        bundle::collect_garbage(&self.bundles_dir(), &state.referenced());
        self.sync_active_dir(&state);
    }

    fn sync_active_dir(&self, state: &State) {
        let dir = state
            .active
            .as_ref()
            .map(|active| self.bundles_dir().join(&active.id));
        if let Ok(mut slot) = self.active_dir.write() {
            *slot = dir;
        }
    }

    fn persist(&self, state: &State) {
        if let Err(error) = state.save(&self.settings.root) {
            eprintln!("forge: ota could not save its state: {error}");
        }
    }

    pub fn status(&self) -> Status {
        let state = self
            .state
            .lock()
            .map(|state| state.clone())
            .unwrap_or_default();
        Status {
            enabled: self.disabled_reason.is_none(),
            disabled_reason: self.disabled_reason.clone(),
            runtime_version: self.settings.runtime_version.clone(),
            app_version: self.settings.app_version.clone(),
            embedded_bundle_id: self.settings.embedded_id.clone(),
            embedded_created_at: self.settings.embedded_created_at,
            active: state.active,
            pending: state.pending,
            last_check: state.last_check,
            last_error: state.last_error,
            last_rollback: state.last_rollback,
            booting: state.booting,
        }
    }

    pub fn mark_healthy(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.booting {
            state.mark_healthy();
            self.persist(&state);
        }
    }

    /// Verifies a manifest and says whether its bundle is worth downloading.
    pub fn consider(&self, raw: &[u8], signature: &str) -> Result<Result<Manifest, Skip>, String> {
        let manifest = Manifest::parse_signed(&self.settings.pubkey, raw, signature)?;
        let state = self.state.lock().map_err(|_| "ota state is poisoned")?;
        let known = state.referenced();
        let verdict = manifest::accept(
            &manifest,
            &Acceptance {
                runtime_version: &self.settings.runtime_version,
                app_version: &self.settings.app_version,
                floor_created_at: state.floor(self.settings.embedded_created_at),
                known_ids: &known,
                blacklist: &state.blacklist,
            },
        );
        Ok(verdict.map(|()| manifest))
    }

    /// Verifies a downloaded bundle against its manifest, unpacks it and marks it pending.
    pub fn install(&self, manifest: &Manifest, zip: &[u8], signature: &str) -> Result<(), String> {
        if zip.len() as u64 != manifest.size {
            return Err("bundle size does not match its manifest".to_string());
        }
        if manifest::sha256_hex(zip) != manifest.sha256.to_ascii_lowercase() {
            return Err("bundle sha256 does not match its manifest".to_string());
        }
        manifest::verify_signature(&self.settings.pubkey, zip, signature)?;
        bundle::unpack(zip, &self.bundles_dir(), &manifest.bundle_id)?;
        let mut state = self.state.lock().map_err(|_| "ota state is poisoned")?;
        state.stage(BundleRef {
            id: manifest.bundle_id.clone(),
            created_at: manifest.created_at,
        });
        self.persist(&state);
        bundle::collect_garbage(&self.bundles_dir(), &state.referenced());
        Ok(())
    }

    fn record_check(&self, error: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            state.last_check = Some(unix_now());
            state.last_error = error;
            self.persist(&state);
        }
    }

    async fn fetch(
        client: &reqwest::Client,
        url: &str,
        cap: u64,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut response = client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("request failed: {error}"))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!("{url} answered {}", response.status()));
        }
        if response.content_length().is_some_and(|length| length > cap) {
            return Err(format!("{url} is larger than {cap} bytes"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("download failed: {error}"))?
        {
            if body.len() as u64 + chunk.len() as u64 > cap {
                return Err(format!("{url} is larger than {cap} bytes"));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Some(body))
    }

    async fn fetch_signed(
        client: &reqwest::Client,
        base: &str,
        asset: &str,
        cap: u64,
    ) -> Result<Option<(Vec<u8>, String)>, String> {
        let Some(body) = Self::fetch(client, &format!("{base}/{asset}"), cap).await? else {
            return Ok(None);
        };
        let signature_asset = manifest::signature_name(asset);
        let Some(signature) = Self::fetch(
            client,
            &format!("{base}/{signature_asset}"),
            SIGNATURE_LIMIT,
        )
        .await?
        else {
            return Err(format!("{signature_asset} is missing"));
        };
        let signature = String::from_utf8(signature).map_err(|_| "signature is not text")?;
        Ok(Some((body, signature)))
    }

    pub async fn check_now(&self) -> Result<Outcome, String> {
        if let Some(reason) = &self.disabled_reason {
            return Err(format!("desktop OTA is off: {reason}"));
        }
        if self.checking.swap(true, Ordering::SeqCst) {
            return Err("an update check is already running".to_string());
        }
        let result = self.check().await;
        self.checking.store(false, Ordering::SeqCst);
        self.record_check(result.as_ref().err().cloned());
        if let Ok(Outcome::Staged(id)) = &result {
            eprintln!("forge: ota bundle {id} is ready; it applies on next launch");
            if let Some(app) = self.app.get() {
                let _ = app.emit(PENDING_EVENT, id);
            }
        }
        result
    }

    async fn check(&self) -> Result<Outcome, String> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(concat!("forge-desktop-ota/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|error| format!("http client: {error}"))?;
        let base = self.settings.base_url.trim_end_matches('/');
        let manifest_asset = manifest::manifest_name(&self.settings.runtime_version);
        let Some((raw, signature)) = Self::fetch_signed(
            &client,
            base,
            &manifest_asset,
            manifest::MAX_MANIFEST_BYTES as u64,
        )
        .await?
        else {
            return Ok(Outcome::NothingPublished);
        };
        let manifest = match self.consider(&raw, &signature)? {
            Ok(manifest) => manifest,
            Err(skip) => return Ok(Outcome::Ignored(skip)),
        };
        let bundle_asset = manifest::bundle_name(&manifest.runtime_version, &manifest.bundle_id);
        let Some((zip, zip_signature)) =
            Self::fetch_signed(&client, base, &bundle_asset, manifest.size).await?
        else {
            return Err(format!("{bundle_asset} is not published yet"));
        };
        self.install(&manifest, &zip, &zip_signature)?;
        Ok(Outcome::Staged(manifest.bundle_id))
    }

    /// Makes the pending bundle live now ("Reload to update"). The caller reloads the webview.
    pub fn apply_pending(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let promoted = state.promote_pending().is_some();
        if promoted {
            self.persist(&state);
            self.sync_active_dir(&state);
        }
        promoted
    }

    pub fn reset(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.reset_to_builtin();
        self.persist(&state);
        self.sync_active_dir(&state);
        bundle::collect_garbage(&self.bundles_dir(), &state.referenced());
    }

    /// The UI never reported healthy: stop serving the bundle that is live.
    fn rollback_if_unhealthy(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.booting || state.active.is_none() {
            return false;
        }
        let Some(bad) = state.abandon_active() else {
            return false;
        };
        state.booting = state.active.is_some();
        eprintln!("forge: ota bundle {bad} did not report healthy in time; rolled back");
        self.persist(&state);
        self.sync_active_dir(&state);
        true
    }

    pub fn spawn(self: &Arc<Self>, app: AppHandle) {
        let _ = self.app.set(app.clone());
        if self.disabled_reason.is_some() {
            return;
        }
        let watchdog = self.clone();
        let watchdog_app = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(BOOT_WATCHDOG).await;
            if watchdog.rollback_if_unhealthy() {
                reload(&watchdog_app);
            }
        });
        let checker = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(FIRST_CHECK_DELAY).await;
            loop {
                if let Err(error) = checker.check_now().await {
                    eprintln!("forge: ota check failed: {error}");
                }
                tokio::time::sleep(CHECK_INTERVAL).await;
            }
        });
    }
}

fn reload(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.eval("window.location.reload()");
    }
}

fn disabled_reason(settings: &Settings) -> Option<String> {
    if cfg!(debug_assertions) {
        return Some("debug build".to_string());
    }
    if std::env::var("FORGE_DESKTOP_OTA").ok().as_deref() == Some("0") {
        return Some("FORGE_DESKTOP_OTA=0".to_string());
    }
    if settings.root.join("disabled").exists() {
        return Some("disabled marker file".to_string());
    }
    if settings.pubkey.is_empty() {
        return Some("no updater public key".to_string());
    }
    None
}

/// Builds the OTA state for this launch and, when enabled, swaps it into the asset pipeline
/// before the webview exists, so the very first page load already comes from the right bundle.
pub fn install_into(context: &mut tauri::Context) -> Arc<Ota> {
    let identifier = context.config().identifier.clone();
    let root = std::env::var_os("FORGE_DESKTOP_OTA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            directories::BaseDirs::new().map(|dirs| dirs.data_dir().join(&identifier).join("ota"))
        })
        .unwrap_or_else(|| std::env::temp_dir().join("forge-ota"));
    let pubkey = context
        .config()
        .plugins
        .0
        .get("updater")
        .and_then(|updater| updater.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let settings = Settings {
        root,
        runtime_version: env!("FORGE_DESKTOP_RUNTIME_VERSION").to_string(),
        app_version: context.package_info().version.to_string(),
        pubkey,
        base_url: std::env::var("FORGE_DESKTOP_OTA_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_string()),
        embedded_id: env!("FORGE_EMBEDDED_BUNDLE_ID").to_string(),
        embedded_created_at: env!("FORGE_EMBEDDED_CREATED_AT").parse().unwrap_or(0),
    };
    let reason = disabled_reason(&settings);
    let ota = Ota::new(settings, reason);
    if ota.disabled_reason.is_none() {
        let embedded = context.set_assets(Box::new(Placeholder));
        context.set_assets(Box::new(OtaAssets::new(embedded, ota.active_dir.clone())));
    }
    ota
}

#[tauri::command]
pub fn ota_status(ota: tauri::State<'_, Arc<Ota>>) -> Status {
    ota.status()
}

#[tauri::command]
pub fn ota_mark_healthy(ota: tauri::State<'_, Arc<Ota>>) {
    ota.mark_healthy();
}

#[tauri::command]
pub async fn ota_check_now(ota: tauri::State<'_, Arc<Ota>>) -> Result<Status, String> {
    let ota = ota.inner().clone();
    ota.check_now().await?;
    Ok(ota.status())
}

#[tauri::command]
pub fn ota_apply_pending(app: AppHandle, ota: tauri::State<'_, Arc<Ota>>) -> bool {
    let applied = ota.apply_pending();
    if applied {
        reload(&app);
    }
    applied
}

#[tauri::command]
pub fn ota_reset(app: AppHandle, ota: tauri::State<'_, Arc<Ota>>) {
    ota.reset();
    reload(&app);
}

#[cfg(test)]
mod tests {
    use super::bundle::test_support::{scratch, zip_of};
    use super::manifest::test_support::{manifest, test_key, TestKey};
    use super::*;

    const RUNTIME: &str = "0123456789abcdef";

    struct Rig {
        root: PathBuf,
        key: TestKey,
    }

    impl Rig {
        fn new(name: &str) -> Rig {
            Rig {
                root: scratch(name),
                key: test_key(),
            }
        }

        fn launch(&self) -> Arc<Ota> {
            Ota::new(
                Settings {
                    root: self.root.clone(),
                    runtime_version: RUNTIME.to_string(),
                    app_version: "2.23.6".to_string(),
                    pubkey: self.key.pubkey_b64.clone(),
                    base_url: String::new(),
                    embedded_id: "embedded".to_string(),
                    embedded_created_at: 100,
                },
                None,
            )
        }

        fn publish(
            &self,
            ota: &Ota,
            runtime: &str,
            id: &str,
            created_at: u64,
            html: &str,
        ) -> Result<Outcome, String> {
            let zip = zip_of(&[("index.html", html.as_bytes())]);
            let manifest = manifest(runtime, id, created_at, &zip);
            let raw = serde_json::to_vec(&manifest).unwrap();
            let signature = self.key.sign(&raw);
            match ota.consider(&raw, &signature)? {
                Err(skip) => Ok(Outcome::Ignored(skip)),
                Ok(manifest) => {
                    ota.install(&manifest, &zip, &self.key.sign(&zip))?;
                    Ok(Outcome::Staged(manifest.bundle_id))
                }
            }
        }

        fn served(&self, ota: &Ota) -> Option<String> {
            let dir = ota.active_dir.read().unwrap().clone()?;
            Some(String::from_utf8(bundle::read(&dir, "/index.html")?.into_owned()).unwrap())
        }
    }

    #[test]
    fn a_staged_bundle_applies_on_the_next_launch_and_survives_when_healthy() {
        let rig = Rig::new("flow");
        let first = rig.launch();
        assert_eq!(rig.served(&first), None);
        assert_eq!(
            rig.publish(&first, RUNTIME, "aaaaaaa1", 200, "v1").unwrap(),
            Outcome::Staged("aaaaaaa1".into())
        );
        assert_eq!(
            rig.served(&first),
            None,
            "nothing changes under a running UI"
        );

        let second = rig.launch();
        assert_eq!(rig.served(&second).as_deref(), Some("v1"));
        second.mark_healthy();
        let third = rig.launch();
        assert_eq!(rig.served(&third).as_deref(), Some("v1"));
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_bundle_that_never_reports_healthy_is_rolled_back_to_the_previous_one() {
        let rig = Rig::new("rollback");
        let ota = rig.launch();
        rig.publish(&ota, RUNTIME, "aaaaaaa1", 200, "good").unwrap();
        let ota = rig.launch();
        ota.mark_healthy();
        rig.publish(&ota, RUNTIME, "bbbbbbb2", 300, "broken")
            .unwrap();

        let broken = rig.launch();
        assert_eq!(rig.served(&broken).as_deref(), Some("broken"));
        let after = rig.launch();
        assert_eq!(rig.served(&after).as_deref(), Some("good"));
        assert_eq!(after.status().last_rollback.as_deref(), Some("bbbbbbb2"));
        assert_eq!(
            rig.publish(&after, RUNTIME, "bbbbbbb2", 300, "broken")
                .unwrap(),
            Outcome::Ignored(Skip::Blacklisted)
        );
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn the_watchdog_rollback_drops_a_bundle_that_hangs_the_current_launch() {
        let rig = Rig::new("watchdog");
        let ota = rig.launch();
        rig.publish(&ota, RUNTIME, "aaaaaaa1", 200, "hangs")
            .unwrap();
        let ota = rig.launch();
        assert_eq!(rig.served(&ota).as_deref(), Some("hangs"));
        assert!(ota.rollback_if_unhealthy());
        assert_eq!(rig.served(&ota), None);
        assert!(!ota.rollback_if_unhealthy());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_newly_installed_release_does_not_serve_bundles_built_for_the_old_one() {
        let rig = Rig::new("upgrade");
        let ota = rig.launch();
        rig.publish(&ota, RUNTIME, "aaaaaaa1", 200, "old shell bundle")
            .unwrap();
        let ota = rig.launch();
        ota.mark_healthy();
        assert_eq!(rig.served(&ota).as_deref(), Some("old shell bundle"));

        let other_runtime = Ota::new(
            Settings {
                root: rig.root.clone(),
                runtime_version: "fedcba9876543210".to_string(),
                app_version: "2.24.0".to_string(),
                pubkey: rig.key.pubkey_b64.clone(),
                base_url: String::new(),
                embedded_id: "embedded".to_string(),
                embedded_created_at: 100,
            },
            None,
        );
        assert_eq!(rig.served(&other_runtime), None);
        assert!(other_runtime.status().active.is_none());
        assert!(!rig.root.join("bundles/aaaaaaa1").exists());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_release_that_embeds_a_newer_ui_replaces_the_bundle() {
        let rig = Rig::new("embeds-newer");
        let ota = rig.launch();
        rig.publish(&ota, RUNTIME, "aaaaaaa1", 200, "bundle")
            .unwrap();
        let ota = rig.launch();
        ota.mark_healthy();
        let newer_release = Ota::new(
            Settings {
                root: rig.root.clone(),
                runtime_version: RUNTIME.to_string(),
                app_version: "2.24.0".to_string(),
                pubkey: rig.key.pubkey_b64.clone(),
                base_url: String::new(),
                embedded_id: "embedded".to_string(),
                embedded_created_at: 250,
            },
            None,
        );
        assert_eq!(rig.served(&newer_release), None);
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_runtime_mismatched_bundle_is_ignored() {
        let rig = Rig::new("runtime");
        let ota = rig.launch();
        assert_eq!(
            rig.publish(&ota, "ffffffffffffffff", "aaaaaaa1", 200, "x")
                .unwrap(),
            Outcome::Ignored(Skip::RuntimeMismatch)
        );
        assert!(ota.status().pending.is_none());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn bundles_are_monotonic_by_created_at() {
        let rig = Rig::new("monotonic");
        let ota = rig.launch();
        assert_eq!(
            rig.publish(&ota, RUNTIME, "aaaaaaa1", 100, "older than embedded")
                .unwrap(),
            Outcome::Ignored(Skip::NotNewer)
        );
        rig.publish(&ota, RUNTIME, "bbbbbbb2", 300, "new").unwrap();
        assert_eq!(
            rig.publish(&ota, RUNTIME, "ccccccc3", 250, "older than pending")
                .unwrap(),
            Outcome::Ignored(Skip::NotNewer)
        );
        let ota = rig.launch();
        ota.mark_healthy();
        assert_eq!(
            rig.publish(&ota, RUNTIME, "ddddddd4", 300, "same age as active")
                .unwrap(),
            Outcome::Ignored(Skip::NotNewer)
        );
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_bundle_that_differs_from_its_manifest_is_rejected() {
        let rig = Rig::new("tamper");
        let ota = rig.launch();
        let zip = zip_of(&[("index.html", b"real")]);
        let manifest = manifest(RUNTIME, "aaaaaaa1", 200, &zip);
        let swapped = zip_of(&[("index.html", b"evil")]);
        assert!(ota
            .install(&manifest, &swapped, &rig.key.sign(&swapped))
            .is_err());

        let mut same_size = manifest.clone();
        same_size.size = zip.len() as u64;
        let forged = rig.key.sign(b"something else");
        assert!(ota.install(&same_size, &zip, &forged).is_err());
        assert!(ota.status().pending.is_none());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn applying_the_pending_bundle_switches_what_is_served_immediately() {
        let rig = Rig::new("apply");
        let ota = rig.launch();
        assert!(!ota.apply_pending());
        rig.publish(&ota, RUNTIME, "aaaaaaa1", 200, "v1").unwrap();
        assert!(ota.apply_pending());
        assert_eq!(rig.served(&ota).as_deref(), Some("v1"));
        assert!(ota.status().booting);
        ota.reset();
        assert_eq!(rig.served(&ota), None);
        assert!(!rig.root.join("bundles/aaaaaaa1").exists());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }

    #[test]
    fn a_disabled_instance_serves_nothing_and_refuses_checks() {
        let rig = Rig::new("disabled");
        let ota = Ota::new(
            Settings {
                root: rig.root.clone(),
                runtime_version: RUNTIME.to_string(),
                app_version: "2.23.6".to_string(),
                pubkey: rig.key.pubkey_b64.clone(),
                base_url: String::new(),
                embedded_id: "embedded".to_string(),
                embedded_created_at: 0,
            },
            Some("FORGE_DESKTOP_OTA=0".to_string()),
        );
        assert!(!ota.status().enabled);
        assert!(tauri::async_runtime::block_on(ota.check_now()).is_err());
        assert!(!rig.root.join("state.json").exists());
        std::fs::remove_dir_all(&rig.root).unwrap();
    }
}
