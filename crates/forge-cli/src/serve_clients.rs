//! Per-client daemon tokens: the registry behind `forge serve pair`, `forge serve clients`, the
//! Remote Access page and the scope checks in [`crate::serve_access`].
//!
//! The legacy `serve-token` stays the owner credential. Every paired device instead holds its own
//! 128-bit token; only its SHA-256 is stored (`serve-clients.json`, 0600), next to a label, the
//! scopes it may use, when it was created and when it was last seen. The CLI and the daemon share
//! the file: the CLI writes it atomically and the daemon re-reads it when its mtime changes, so a
//! `revoke` from a terminal cuts a live connection within a couple of seconds.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub(crate) const CLIENTS_FILE: &str = "serve-clients.json";
pub(crate) const DEFAULT_PAIR_TTL_SECS: u64 = 600;
pub(crate) const MAX_PAIR_TTL_SECS: u64 = 86_400;
const MAX_CLIENTS: usize = 64;
const MAX_TICKETS: usize = 256;
const RELOAD_EVERY: Duration = Duration::from_secs(1);
const FLUSH_EVERY: Duration = Duration::from_secs(30);
const TICKET_TTL: Duration = Duration::from_secs(30);
const REVOCATION_POLL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scope {
    View,
    Prompt,
    Approve,
    Terminal,
    WorkspaceWrite,
    ManageProviders,
}

impl Scope {
    pub(crate) const ALL: [Scope; 6] = [
        Scope::View,
        Scope::Prompt,
        Scope::Approve,
        Scope::Terminal,
        Scope::WorkspaceWrite,
        Scope::ManageProviders,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Scope::View => "view",
            Scope::Prompt => "prompt",
            Scope::Approve => "approve",
            Scope::Terminal => "terminal",
            Scope::WorkspaceWrite => "workspace-write",
            Scope::ManageProviders => "manage-providers",
        }
    }
}

/// What a freshly paired device may do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Preset {
    Full,
    Viewer,
}

impl Preset {
    pub(crate) fn parse(text: &str) -> Option<Preset> {
        match text.trim().to_ascii_lowercase().as_str() {
            "full" | "owner" | "operator" => Some(Preset::Full),
            "viewer" | "view" | "view-only" => Some(Preset::Viewer),
            _ => None,
        }
    }

    pub(crate) fn scopes(self) -> Vec<Scope> {
        match self {
            Preset::Full => Scope::ALL.to_vec(),
            Preset::Viewer => vec![Scope::View],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ClientRecord {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) token_hash: String,
    pub(crate) scopes: Vec<Scope>,
    pub(crate) created_at: u64,
    #[serde(default)]
    pub(crate) last_seen: Option<u64>,
    /// Set while the pairing link has not been used yet; past it the token is dead and swept.
    #[serde(default)]
    pub(crate) pending_until: Option<u64>,
}

#[derive(Serialize, Deserialize, Default)]
struct ClientsFile {
    #[serde(default)]
    clients: Vec<ClientRecord>,
}

/// Who a request belongs to once the gate has resolved its token.
#[derive(Clone, Debug)]
pub(crate) struct ClientAuth {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) scopes: Vec<Scope>,
}

impl ClientAuth {
    pub(crate) fn owner() -> Self {
        Self {
            id: None,
            label: "owner".into(),
            scopes: Scope::ALL.to_vec(),
        }
    }

    pub(crate) fn is_owner(&self) -> bool {
        self.id.is_none()
    }

    pub(crate) fn allows(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }

    pub(crate) fn name(&self) -> &str {
        self.id.as_deref().unwrap_or("owner")
    }
}

struct Ticket {
    value: String,
    auth: ClientAuth,
    issued: Instant,
}

struct Inner {
    clients: Vec<ClientRecord>,
    mtime: Option<SystemTime>,
    checked: Instant,
    flushed: Instant,
    dirty: bool,
    tickets: Vec<Ticket>,
}

pub(crate) struct ClientRegistry {
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    revoked: tokio::sync::watch::Sender<u64>,
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) fn looks_like_client_token(segment: &str) -> bool {
    segment.len() == 32 && segment.bytes().all(|b| b.is_ascii_hexdigit())
}

fn random_hex(words: usize) -> String {
    (0..words)
        .map(|_| format!("{:016x}", rand::random::<u64>()))
        .collect()
}

impl ClientRegistry {
    pub(crate) fn open(path: Option<PathBuf>) -> Self {
        let (clients, mtime) = match &path {
            Some(p) => (read_file(p).unwrap_or_default(), file_mtime(p)),
            None => (Vec::new(), None),
        };
        Self {
            path,
            inner: Mutex::new(Inner {
                clients,
                mtime,
                checked: Instant::now(),
                flushed: Instant::now(),
                dirty: false,
                tickets: Vec::new(),
            }),
            revoked: tokio::sync::watch::channel(0).0,
        }
    }

    pub(crate) fn default_path() -> Option<PathBuf> {
        forge_config::config_dir().map(|d| d.join(CLIENTS_FILE))
    }

    /// A file next to the clients file, so state that belongs to paired clients lives with them.
    pub(crate) fn sibling_path(&self, name: &str) -> Option<PathBuf> {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .map(|d| d.join(name))
    }

    pub(crate) fn active_ids(&self) -> std::collections::HashSet<String> {
        let mut inner = self.lock();
        self.refresh(&mut inner, true);
        inner.clients.iter().map(|c| c.id.clone()).collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Pick up edits another process (the CLI) made to the file, keeping the newest `last_seen`
    /// this process has seen for each client.
    fn refresh(&self, inner: &mut Inner, force: bool) {
        let Some(path) = &self.path else { return };
        if !force && inner.checked.elapsed() < RELOAD_EVERY {
            return;
        }
        inner.checked = Instant::now();
        let mtime = file_mtime(path);
        if mtime == inner.mtime {
            return;
        }
        let Some(mut disk) = read_file(path) else {
            return;
        };
        for record in &mut disk {
            if let Some(mine) = inner.clients.iter().find(|c| c.id == record.id) {
                record.last_seen = record.last_seen.max(mine.last_seen);
            }
        }
        let removed = inner
            .clients
            .iter()
            .any(|c| !disk.iter().any(|d| d.id == c.id));
        inner.clients = disk;
        inner.mtime = mtime;
        if removed {
            self.revoked.send_modify(|n| *n += 1);
        }
    }

    fn persist(&self, inner: &mut Inner) -> Result<()> {
        inner.dirty = false;
        inner.flushed = Instant::now();
        let Some(path) = &self.path else {
            return Ok(());
        };
        let body = serde_json::to_vec_pretty(&ClientsFile {
            clients: inner.clients.clone(),
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        inner.mtime = file_mtime(path);
        Ok(())
    }

    fn sweep(inner: &mut Inner) -> bool {
        let now = now_secs();
        let before = inner.clients.len();
        inner
            .clients
            .retain(|c| c.pending_until.is_none_or(|until| until >= now));
        before != inner.clients.len()
    }

    /// Resolve a client token to its identity, marking a pending pairing as claimed.
    pub(crate) fn authenticate(&self, token: &str) -> Option<ClientAuth> {
        if !looks_like_client_token(token) {
            return None;
        }
        let hash = hash_token(token);
        let mut inner = self.lock();
        self.refresh(&mut inner, false);
        let now = now_secs();
        let record = inner.clients.iter_mut().find(|c| c.token_hash == hash)?;
        let mut claimed = false;
        if let Some(until) = record.pending_until {
            if now > until {
                return None;
            }
            record.pending_until = None;
            claimed = true;
        }
        record.last_seen = Some(now);
        let auth = ClientAuth {
            id: Some(record.id.clone()),
            label: record.label.clone(),
            scopes: record.scopes.clone(),
        };
        inner.dirty = true;
        if claimed || inner.flushed.elapsed() >= FLUSH_EVERY {
            let _ = self.persist(&mut inner);
        }
        Some(auth)
    }

    /// Mint a client token. With `pending_ttl` the link must be used within that many seconds.
    pub(crate) fn mint(
        &self,
        label: &str,
        scopes: Vec<Scope>,
        pending_ttl: Option<u64>,
    ) -> Result<(ClientRecord, String)> {
        let mut inner = self.lock();
        self.refresh(&mut inner, true);
        Self::sweep(&mut inner);
        if inner.clients.len() >= MAX_CLIENTS {
            bail!("too many paired devices ({MAX_CLIENTS}); revoke some first");
        }
        let token = random_hex(2);
        let mut id = random_hex(1)[..8].to_string();
        while inner.clients.iter().any(|c| c.id == id) {
            id = random_hex(1)[..8].to_string();
        }
        let now = now_secs();
        let label = label.trim();
        let label = if label.is_empty() {
            format!("Device {}", inner.clients.len() + 1)
        } else {
            label.chars().take(64).collect()
        };
        let record = ClientRecord {
            id,
            label,
            token_hash: hash_token(&token),
            scopes,
            created_at: now,
            last_seen: None,
            pending_until: pending_ttl.map(|ttl| now + ttl.clamp(30, MAX_PAIR_TTL_SECS)),
        };
        inner.clients.push(record.clone());
        self.persist(&mut inner)?;
        Ok((record, token))
    }

    /// Remove a client by id or unambiguous prefix and cut its open connections.
    pub(crate) fn revoke(&self, id: &str) -> Result<ClientRecord> {
        let mut inner = self.lock();
        self.refresh(&mut inner, true);
        let id = id.trim();
        let matches: Vec<usize> = inner
            .clients
            .iter()
            .enumerate()
            .filter(|(_, c)| !id.is_empty() && c.id.starts_with(id))
            .map(|(i, _)| i)
            .collect();
        let index = match matches.as_slice() {
            [one] => *one,
            [] => bail!("no paired device matches `{id}`"),
            _ => bail!("`{id}` matches several devices; use more of the id"),
        };
        let record = inner.clients.remove(index);
        self.persist(&mut inner)?;
        self.revoked.send_modify(|n| *n += 1);
        Ok(record)
    }

    pub(crate) fn list(&self) -> Vec<ClientRecord> {
        let mut inner = self.lock();
        self.refresh(&mut inner, true);
        if Self::sweep(&mut inner) {
            let _ = self.persist(&mut inner);
        }
        inner.clients.clone()
    }

    fn is_active(&self, id: &str) -> bool {
        let mut inner = self.lock();
        self.refresh(&mut inner, false);
        inner.clients.iter().any(|c| c.id == id)
    }

    /// Resolves once `auth`'s client has been revoked; never for the owner.
    pub(crate) async fn until_revoked(&self, auth: &ClientAuth) {
        let Some(id) = auth.id.as_deref() else {
            return std::future::pending().await;
        };
        let mut rx = self.revoked.subscribe();
        while self.is_active(id) {
            tokio::select! {
                _ = rx.changed() => {}
                _ = tokio::time::sleep(REVOCATION_POLL) => {}
            }
        }
    }

    /// A single-use, 30 second stand-in for the bearer on a WebSocket URL.
    pub(crate) fn issue_ticket(&self, auth: &ClientAuth) -> String {
        let value = random_hex(2);
        let mut inner = self.lock();
        inner.tickets.retain(|t| t.issued.elapsed() < TICKET_TTL);
        if inner.tickets.len() >= MAX_TICKETS {
            inner.tickets.remove(0);
        }
        inner.tickets.push(Ticket {
            value: value.clone(),
            auth: auth.clone(),
            issued: Instant::now(),
        });
        value
    }

    pub(crate) fn redeem_ticket(&self, ticket: &str) -> Option<ClientAuth> {
        let auth = {
            let mut inner = self.lock();
            inner.tickets.retain(|t| t.issued.elapsed() < TICKET_TTL);
            let index = inner.tickets.iter().position(|t| t.value == ticket)?;
            inner.tickets.remove(index).auth
        };
        match auth.id.as_deref() {
            Some(id) if !self.is_active(id) => None,
            _ => Some(auth),
        }
    }
}

fn file_mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read_file(path: &std::path::Path) -> Option<Vec<ClientRecord>> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<ClientsFile>(&text)
        .ok()
        .map(|f| f.clients)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_registry(name: &str) -> (ClientRegistry, PathBuf) {
        let dir = std::env::temp_dir().join(format!("forge-clients-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(CLIENTS_FILE);
        (ClientRegistry::open(Some(path.clone())), path)
    }

    #[test]
    fn minted_token_authenticates_and_only_its_hash_is_stored() {
        let (reg, path) = temp_registry("hash");
        let (record, token) = reg.mint("Pixel", Preset::Full.scopes(), None).unwrap();
        let auth = reg.authenticate(&token).expect("token resolves");
        assert_eq!(auth.id.as_deref(), Some(record.id.as_str()));
        assert!(auth.allows(Scope::ManageProviders));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains(&token), "raw token never reaches disk");
        assert!(on_disk.contains(&hash_token(&token)));
        assert!(reg.authenticate(&"0".repeat(32)).is_none());
    }

    #[test]
    fn revoking_one_client_leaves_the_other_working() {
        let (reg, _) = temp_registry("revoke");
        let (a, ta) = reg.mint("A", Preset::Full.scopes(), None).unwrap();
        let (_, tb) = reg.mint("B", Preset::Full.scopes(), None).unwrap();
        reg.revoke(&a.id).unwrap();
        assert!(reg.authenticate(&ta).is_none());
        assert!(reg.authenticate(&tb).is_some());
        assert!(reg.revoke(&a.id).is_err());
    }

    #[test]
    fn viewer_preset_holds_only_view() {
        let (reg, _) = temp_registry("viewer");
        let (_, token) = reg.mint("TV", Preset::Viewer.scopes(), None).unwrap();
        let auth = reg.authenticate(&token).unwrap();
        assert!(auth.allows(Scope::View));
        assert!(!auth.allows(Scope::Prompt) && !auth.allows(Scope::Approve));
    }

    #[test]
    fn unclaimed_pairing_expires_and_a_claimed_one_does_not() {
        let (reg, path) = temp_registry("pending");
        let (stale, stale_token) = reg.mint("late", Preset::Full.scopes(), Some(30)).unwrap();
        let (_, fresh_token) = reg.mint("now", Preset::Full.scopes(), Some(600)).unwrap();
        let mut file: ClientsFile =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        file.clients
            .iter_mut()
            .find(|c| c.id == stale.id)
            .unwrap()
            .pending_until = Some(now_secs() - 5);
        std::fs::write(&path, serde_json::to_string(&file).unwrap()).unwrap();
        reg.refresh(&mut reg.lock(), true);
        assert!(reg.authenticate(&stale_token).is_none());
        assert!(reg.authenticate(&fresh_token).is_some());
        assert!(reg.list().iter().all(|c| c.id != stale.id), "expired swept");
        let claimed = reg.list().into_iter().find(|c| c.label == "now").unwrap();
        assert!(claimed.pending_until.is_none());
    }

    #[test]
    fn cli_revocation_reaches_a_running_registry() {
        let (daemon, path) = temp_registry("shared");
        let (record, token) = daemon.mint("phone", Preset::Full.scopes(), None).unwrap();
        assert!(daemon.authenticate(&token).is_some());
        let cli = ClientRegistry::open(Some(path));
        cli.revoke(&record.id[..4]).unwrap();
        daemon.refresh(&mut daemon.lock(), true);
        assert!(daemon.authenticate(&token).is_none());
    }

    #[test]
    fn tickets_are_single_use() {
        let (reg, _) = temp_registry("ticket");
        let (_, token) = reg.mint("w", Preset::Viewer.scopes(), None).unwrap();
        let auth = reg.authenticate(&token).unwrap();
        let ticket = reg.issue_ticket(&auth);
        assert_eq!(reg.redeem_ticket(&ticket).unwrap().id, auth.id);
        assert!(reg.redeem_ticket(&ticket).is_none());
        assert!(reg.redeem_ticket("nope").is_none());
    }

    #[tokio::test]
    async fn until_revoked_resolves_after_revoke() {
        let (reg, _) = temp_registry("until");
        let reg = std::sync::Arc::new(reg);
        let (record, token) = reg.mint("w", Preset::Full.scopes(), None).unwrap();
        let auth = reg.authenticate(&token).unwrap();
        let waiter = {
            let reg = reg.clone();
            tokio::spawn(async move { reg.until_revoked(&auth).await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished());
        reg.revoke(&record.id).unwrap();
        tokio::time::timeout(Duration::from_secs(3), waiter)
            .await
            .expect("revocation observed")
            .unwrap();
    }
}
