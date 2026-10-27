//! Serves the web UI from the active OTA bundle, falling back to the embedded assets.
//!
//! Installed as Tauri's `Assets`, so the UI keeps the same `tauri://localhost` origin (and with it
//! localStorage, IndexedDB and the IPC capabilities) whichever bundle is live.

use super::bundle;
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::utils::assets::{AssetKey, AssetsIter, CspHash};
use tauri::{Assets, Runtime};

pub type ActiveDir = Arc<RwLock<Option<PathBuf>>>;

pub struct OtaAssets<R: Runtime> {
    embedded: Box<dyn Assets<R>>,
    active: ActiveDir,
}

impl<R: Runtime> OtaAssets<R> {
    pub fn new(embedded: Box<dyn Assets<R>>, active: ActiveDir) -> Self {
        Self { embedded, active }
    }
}

impl<R: Runtime> Assets<R> for OtaAssets<R> {
    fn setup(&self, app: &tauri::App<R>) {
        self.embedded.setup(app);
    }

    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        let active = self.active.read().ok().and_then(|dir| dir.clone());
        match active {
            // A live bundle never borrows from the embedded UI: mixing the two would pair a new
            // index.html with old chunks.
            Some(dir) => bundle::read(&dir, key.as_ref()),
            None => self.embedded.get(key),
        }
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        self.embedded.iter()
    }

    fn csp_hashes(&self, html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        self.embedded.csp_hashes(html_path)
    }
}

/// Stands in for the embedded assets while `OtaAssets` is being built around them.
pub struct Placeholder;

impl<R: Runtime> Assets<R> for Placeholder {
    fn get(&self, _key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        None
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(std::iter::empty())
    }

    fn csp_hashes(&self, _html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ota::bundle::test_support::{scratch, zip_of};
    use tauri::test::MockRuntime;

    struct Embedded;
    impl Assets<MockRuntime> for Embedded {
        fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
            (key.as_ref() == "/index.html").then(|| Cow::Borrowed(&b"embedded"[..]))
        }
        fn iter(&self) -> Box<AssetsIter<'_>> {
            Box::new(std::iter::empty())
        }
        fn csp_hashes(&self, _html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
            Box::new(std::iter::empty())
        }
    }

    #[test]
    fn serves_the_embedded_ui_until_a_bundle_is_active_and_follows_switches() {
        let dir = scratch("assets");
        let target =
            bundle::unpack(&zip_of(&[("index.html", b"from bundle")]), &dir, "abcdef01").unwrap();
        let active: ActiveDir = Arc::new(RwLock::new(None));
        let assets = OtaAssets::<MockRuntime>::new(Box::new(Embedded), active.clone());
        let index = AssetKey::from("/index.html");
        let other = AssetKey::from("/other.js");

        assert_eq!(assets.get(&index).unwrap().as_ref(), b"embedded");
        *active.write().unwrap() = Some(target);
        assert_eq!(assets.get(&index).unwrap().as_ref(), b"from bundle");
        assert!(
            assets.get(&other).is_none(),
            "no fallthrough into the embedded UI"
        );
        *active.write().unwrap() = None;
        assert_eq!(assets.get(&index).unwrap().as_ref(), b"embedded");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
