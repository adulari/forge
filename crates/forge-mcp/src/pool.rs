//! Server connections shared between managers.
//!
//! The daemon builds one [`McpManager`](crate::McpManager) per session, and every restored session
//! used to open its own connection to every configured server: three sessions meant three
//! concurrent connects to Helm at boot, and N idle connections for as long as the daemon lived.
//! A [`McpPool`] hands every manager the same [`SharedServer`] for a given server definition, so
//! the connect happens once (concurrent callers wait for it instead of starting their own), the
//! idle cost is one connection, and a drop is repaired once for everybody.
//!
//! What stays per session: the manager's own catalog mirror, status, call timeouts, tool
//! allowlist and `reconnect_attempts`. Permission gating lives in `Session::invoke_tool`, above
//! this crate, and never sees the pool.
//!
//! Only servers that [`shares_connection`](forge_config::McpServerConfig::shares_connection) are
//! pooled: HTTP/SSE by default, stdio only on an explicit `shared = true`. A stdio server is a
//! child process whose cwd, environment and in-memory state belong to whoever spawned it, so
//! sharing it would let one session's state leak into another.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use forge_config::McpServerConfig;
use parking_lot::Mutex;
use rmcp::service::{Peer, RoleClient, RunningService};

use crate::transport::{self, Established, HandlerDeps};
use crate::{
    Conns, DiscoveredPrompt, DiscoveredResource, DiscoveredTool, ForgeClientHandler, ServerStatus,
};

/// A connect that just failed is reported as-is to anyone who asks within this window, instead of
/// every waiting session re-running (and re-timing-out) the same doomed handshake in turn.
const FAILURE_COALESCE: Duration = Duration::from_secs(2);

pub(crate) type ConnectFuture =
    Pin<Box<dyn Future<Output = Result<Established, String>> + Send + 'static>>;
pub(crate) type Connector =
    Arc<dyn Fn(McpServerConfig, HandlerDeps) -> ConnectFuture + Send + Sync>;

fn real_connector() -> Connector {
    Arc::new(|server, deps| Box::pin(async move { transport::serve(&server, deps).await }))
}

/// How a handler reaches the catalog it must refresh on `tools/list_changed`.
pub(crate) enum CatalogLink {
    /// No catalog to refresh (in-process test handler).
    None,
    /// One manager's connection map.
    Manager(std::sync::Weak<Conns>),
    /// A pooled server, which fans the refresh out to every manager using it.
    Shared(Weak<SharedServer>),
}

/// A snapshot of a pooled server's connection and catalog. A reconnect replaces it wholesale;
/// `generation` tells a caller whether the connection it saw die is still the current one.
pub(crate) struct Live {
    pub(crate) generation: u64,
    pub(crate) peer: Peer<RoleClient>,
    pub(crate) tools: Vec<DiscoveredTool>,
    pub(crate) resources: Vec<DiscoveredResource>,
    pub(crate) prompts: Vec<DiscoveredPrompt>,
}

#[derive(Default)]
struct Gate {
    service: Option<RunningService<RoleClient, ForgeClientHandler>>,
    child_group: Option<u32>,
    last_failure: Option<(Instant, String)>,
}

/// One connection to one server definition, used by every manager that leased it. Dropping the
/// last lease closes the connection (and signals a stdio child tree).
pub(crate) struct SharedServer {
    server: McpServerConfig,
    connect_timeout: Duration,
    roots: Vec<rmcp::model::Root>,
    connector: Connector,
    /// Serializes connect and reconnect, and owns what must be torn down with the connection.
    gate: tokio::sync::Mutex<Gate>,
    live: Mutex<Option<Arc<Live>>>,
    generation: AtomicU64,
    subscribers: Mutex<Vec<Weak<Conns>>>,
}

impl Drop for SharedServer {
    fn drop(&mut self) {
        if let Some(group) = self.gate.get_mut().child_group.take() {
            transport::signal_child_group(group);
        }
    }
}

impl SharedServer {
    pub(crate) fn server(&self) -> &McpServerConfig {
        &self.server
    }

    pub(crate) fn current(&self) -> Option<Arc<Live>> {
        self.live.lock().clone()
    }

    /// Register a manager's connection map so connects, drops and catalog changes reach it.
    pub(crate) fn subscribe(&self, conns: &Arc<Conns>) {
        let mut subs = self.subscribers.lock();
        subs.retain(|w| w.strong_count() > 0);
        if !subs
            .iter()
            .any(|w| std::ptr::eq(w.as_ptr(), Arc::as_ptr(conns)))
        {
            subs.push(Arc::downgrade(conns));
        }
    }

    fn deps(self: &Arc<Self>) -> HandlerDeps {
        HandlerDeps {
            roots: self.roots.clone(),
            sampling: None,
            conns: CatalogLink::Shared(Arc::downgrade(self)),
        }
    }

    /// The live connection, connecting first if there is none. Concurrent callers queue on the
    /// gate, so the handshake runs once and the rest return its result.
    pub(crate) async fn ensure(self: &Arc<Self>) -> Result<Arc<Live>, String> {
        if let Some(live) = self.current() {
            return Ok(live);
        }
        let mut gate = self.gate.lock().await;
        if let Some(live) = self.current() {
            return Ok(live);
        }
        self.establish(&mut gate).await
    }

    /// Replace a connection that a caller saw die. If another session already did, or the
    /// connection was never the dead one, that newer connection is returned untouched.
    pub(crate) async fn refresh(
        self: &Arc<Self>,
        dead_generation: u64,
    ) -> Result<Arc<Live>, String> {
        let mut gate = self.gate.lock().await;
        if let Some(live) = self.current().filter(|l| l.generation > dead_generation) {
            return Ok(live);
        }
        *self.live.lock() = None;
        if let Some(group) = gate.child_group.take() {
            transport::signal_child_group(group);
        }
        if let Some(old) = gate.service.take() {
            tokio::spawn(async move {
                let _ = old.cancel().await;
            });
        }
        self.establish(&mut gate).await
    }

    async fn establish(self: &Arc<Self>, gate: &mut Gate) -> Result<Arc<Live>, String> {
        if let Some((at, reason)) = &gate.last_failure {
            if at.elapsed() < FAILURE_COALESCE {
                return Err(reason.clone());
            }
        }
        let name = self.server.name.clone();
        let connect = async {
            let established = (self.connector)(self.server.clone(), self.deps()).await?;
            let peer = established.service.peer().clone();
            let (tools, resources, prompts) = discover_all(&peer, &name).await;
            Ok::<_, String>((established, peer, tools, resources, prompts))
        };
        let outcome = match tokio::time::timeout(self.connect_timeout, connect).await {
            Ok(result) => result,
            Err(_) => Err(format!(
                "connect/initialize/discovery timed out after {}s",
                self.connect_timeout.as_secs()
            )),
        };
        match outcome {
            Ok((established, peer, tools, resources, prompts)) => {
                let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
                gate.service = Some(established.service);
                gate.child_group = established.child_group;
                gate.last_failure = None;
                let live = Arc::new(Live {
                    generation,
                    peer,
                    tools,
                    resources,
                    prompts,
                });
                *self.live.lock() = Some(Arc::clone(&live));
                self.publish();
                Ok(live)
            }
            Err(reason) => {
                gate.last_failure = Some((Instant::now(), reason.clone()));
                Err(reason)
            }
        }
    }

    /// A call on `generation` failed at the transport. Withdraw that connection (a newer one is
    /// left alone) so the next call on any session reconnects once.
    pub(crate) fn mark_dead(self: &Arc<Self>, generation: u64) {
        {
            let mut live = self.live.lock();
            if live.as_ref().is_none_or(|l| l.generation != generation) {
                return;
            }
            *live = None;
        }
        self.for_each_mirror(|c| {
            c.peer = None;
            c.status = ServerStatus::Reconnecting;
        });
    }

    /// Push the current connection and catalog into every subscribed manager's mirror.
    fn publish(self: &Arc<Self>) {
        let Some(live) = self.current() else { return };
        self.for_each_mirror(|c| c.adopt(&live));
    }

    /// A `tools/list_changed` for this server: swap the tools into the live snapshot and every
    /// mirror.
    pub(crate) fn update_tools(self: &Arc<Self>, tools: Vec<DiscoveredTool>) {
        {
            let mut live = self.live.lock();
            let Some(current) = live.as_ref() else { return };
            *live = Some(Arc::new(Live {
                generation: current.generation,
                peer: current.peer.clone(),
                tools: tools.clone(),
                resources: current.resources.clone(),
                prompts: current.prompts.clone(),
            }));
        }
        self.for_each_mirror(|c| c.tools = tools.clone());
    }

    fn for_each_mirror(self: &Arc<Self>, mut apply: impl FnMut(&mut crate::Connection)) {
        let maps: Vec<Arc<Conns>> = {
            let mut subs = self.subscribers.lock();
            subs.retain(|w| w.strong_count() > 0);
            subs.iter().filter_map(Weak::upgrade).collect()
        };
        for conns in maps {
            if let Some(c) = conns.lock().get_mut(&self.server.name) {
                if c.shared.as_ref().is_some_and(|s| Arc::ptr_eq(s, self)) {
                    apply(c);
                }
            }
        }
    }
}

impl crate::Connection {
    /// Take on a pooled connection's current peer and catalog.
    fn adopt(&mut self, live: &Live) {
        self.status = ServerStatus::Connected;
        self.peer = Some(live.peer.clone());
        self.tools = live.tools.clone();
        self.resources = live.resources.clone();
        self.prompts = live.prompts.clone();
        self.reconnect_attempts = 0;
        self.shared_gen = live.generation;
    }
}

impl crate::McpManager {
    /// Connect shareable servers through `pool`, so every manager built over the same pool shares
    /// one connection per server. Builder-style; call before connecting.
    pub fn with_pool(mut self, pool: Arc<McpPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    /// The pooled connection for `server`, if this manager pools it. A manager with a sampling
    /// handler never does: sampling answers on behalf of one session's model and budget.
    fn shared_entry_for(
        &self,
        server: &forge_config::McpServerConfig,
    ) -> Option<Arc<SharedServer>> {
        let pool = self.pool.as_ref()?;
        (self.sampling.is_none() && server.shares_connection())
            .then(|| pool.server_for(server, &self.roots, self.connect_timeout))
    }

    /// Install (or refresh) this manager's mirror of a pooled server from its current connection.
    pub(crate) fn install_shared(&self, entry: &Arc<SharedServer>) {
        let Some(live) = entry.current() else { return };
        let server = entry.server();
        let mut conns = self.conns.lock();
        let c = conns
            .entry(server.name.clone())
            .or_insert_with(|| crate::Connection {
                name: server.name.clone(),
                status: ServerStatus::Reconnecting,
                transport_label: server.transport_label(),
                peer: None,
                service: None,
                child_group: None,
                tools: vec![],
                resources: vec![],
                prompts: vec![],
                reconnect_attempts: 0,
                shared: None,
                shared_gen: 0,
            });
        c.shared = Some(Arc::clone(entry));
        c.adopt(&live);
    }

    /// Connect one server: through the pool when it is shared, privately otherwise.
    pub(crate) async fn establish_server(
        &self,
        server: &forge_config::McpServerConfig,
    ) -> Result<(), String> {
        if let Some(entry) = self.shared_entry_for(server) {
            entry.subscribe(&self.conns);
            entry.ensure().await?;
            self.install_shared(&entry);
            return Ok(());
        }
        let service = transport::serve(server, self.handler_deps()).await?;
        self.add_established(&server.name, server.transport_label(), service)
            .await;
        Ok(())
    }
}

/// A server's whole catalog: tools (namespaced), resources and prompts. Listing failures leave that
/// part empty rather than failing the connection.
pub(crate) async fn discover_all(
    peer: &Peer<RoleClient>,
    server: &str,
) -> (
    Vec<DiscoveredTool>,
    Vec<DiscoveredResource>,
    Vec<DiscoveredPrompt>,
) {
    let tools = crate::discover_tools(peer, server).await;
    let resources = peer
        .list_all_resources()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|r| DiscoveredResource {
            uri: r.uri.clone(),
            name: r.name.clone(),
            mime: r.mime_type.clone(),
        })
        .collect();
    let prompts = peer
        .list_all_prompts()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| DiscoveredPrompt {
            name: p.name.clone(),
            description: p.description.clone().unwrap_or_default(),
        })
        .collect();
    (tools, resources, prompts)
}

/// The set of live pooled servers, keyed by complete server definition.
pub struct McpPool {
    entries: Mutex<HashMap<String, Weak<SharedServer>>>,
    connector: Connector,
}

impl Default for McpPool {
    fn default() -> Self {
        Self::new()
    }
}

impl McpPool {
    pub fn new() -> Self {
        Self::with_connector(real_connector())
    }

    pub(crate) fn with_connector(connector: Connector) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            connector,
        }
    }

    /// The process-wide pool the daemon's sessions share.
    pub fn global() -> Arc<McpPool> {
        static GLOBAL: OnceLock<Arc<McpPool>> = OnceLock::new();
        Arc::clone(GLOBAL.get_or_init(|| Arc::new(McpPool::new())))
    }

    /// Servers currently held open by at least one manager.
    pub fn live_servers(&self) -> usize {
        self.entries
            .lock()
            .values()
            .filter(|w| w.strong_count() > 0)
            .count()
    }

    /// The pooled connection for `server`, created (but not yet connected) on first use. The key
    /// is the whole serialized definition plus the roots advertised to it, so two sessions share
    /// a connection exactly when they would otherwise have opened identical ones.
    pub(crate) fn server_for(
        &self,
        server: &McpServerConfig,
        roots: &[rmcp::model::Root],
        connect_timeout: Duration,
    ) -> Arc<SharedServer> {
        let key = format!(
            "{}\u{1}{}",
            serde_json::to_string(server).unwrap_or_else(|_| server.name.clone()),
            roots
                .iter()
                .map(|r| r.uri.as_str())
                .collect::<Vec<_>>()
                .join("\u{2}")
        );
        let mut entries = self.entries.lock();
        entries.retain(|_, w| w.strong_count() > 0);
        if let Some(existing) = entries.get(&key).and_then(Weak::upgrade) {
            return existing;
        }
        let created = Arc::new(SharedServer {
            server: server.clone(),
            connect_timeout,
            roots: roots.to_vec(),
            connector: Arc::clone(&self.connector),
            gate: tokio::sync::Mutex::new(Gate::default()),
            live: Mutex::new(None),
            generation: AtomicU64::new(0),
            subscribers: Mutex::new(Vec::new()),
        });
        entries.insert(key, Arc::downgrade(&created));
        created
    }
}

#[cfg(test)]
mod tests;
