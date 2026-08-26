pub(crate) mod assay;
pub(crate) mod blame;
pub(crate) mod git;
pub(crate) mod import;
pub(crate) mod lattice;
pub(crate) mod local;
pub(crate) mod marketplace;
pub(crate) mod mcp;
pub(crate) mod memory;
pub(crate) mod migrate;
pub(crate) mod models;
pub(crate) mod oauth_flow;
pub(crate) mod provider;
pub(crate) mod queue;
pub(crate) mod replay;
pub(crate) mod run;
pub(crate) mod schedule;
pub(crate) mod self_mcp;
pub(crate) mod send;
pub(crate) mod service;
pub(crate) mod skill;
#[cfg(test)]
mod tests;
pub(crate) mod tour;
pub(crate) mod voice;

use anyhow::{Context, Result};

/// Load the effective CLI configuration without turning a present parse failure into defaults.
/// Commands should use this boundary so users see the source path and underlying error instead of
/// unknowingly running with different provider, routing, or local-runtime settings.
pub(crate) fn load_config() -> Result<forge_config::Config> {
    forge_config::load().context("loading Forge configuration")
}
