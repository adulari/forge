//! Late catalog discovery: lets a router built before a provider was reachable pick up the
//! complete catalog once a retried discovery succeeds.

use std::sync::Arc;

use crate::catalog::ModelCatalog;
use crate::HeuristicRouter;

/// Shared slot a background re-discovery publishes into so routers built before a provider became
/// reachable switch to the complete catalog without a restart. Cheap to clone; every clone sees
/// the same slot.
#[derive(Debug, Clone, Default)]
pub struct LiveCatalog(Arc<std::sync::RwLock<Option<Arc<ModelCatalog>>>>);

impl LiveCatalog {
    pub fn publish(&self, catalog: ModelCatalog) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(catalog));
    }

    pub fn current(&self) -> Option<Arc<ModelCatalog>> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl HeuristicRouter {
    /// Follow `live`: once a late discovery publishes a catalog there, it replaces the one this
    /// router was built with.
    pub fn with_live_catalog(mut self, live: LiveCatalog) -> Self {
        self.live_catalog = Some(live);
        self
    }

    /// The catalog routing ranks against right now: a published late discovery if any, else the
    /// one the router was built with.
    pub(crate) fn catalog(&self) -> Option<Arc<ModelCatalog>> {
        self.live_catalog
            .as_ref()
            .and_then(LiveCatalog::current)
            .or_else(|| self.catalog.clone())
    }
}
