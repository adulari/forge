//! The mutable parts of a report's context: what the live session is running on.

use std::sync::Mutex;

use crate::report::{self, Context, Surface};
use crate::HOST;

static SESSION: Mutex<SessionCtx> = Mutex::new(SessionCtx::new());

struct SessionCtx {
    provider: Option<String>,
    model: Option<String>,
    temper: Option<String>,
    flags: Vec<String>,
}

impl SessionCtx {
    const fn new() -> Self {
        SessionCtx {
            provider: None,
            model: None,
            temper: None,
            flags: Vec::new(),
        }
    }
}

/// Provider, model and temper of the live session (ids only, never keys).
pub fn set_session_context(provider: Option<&str>, model: Option<&str>, temper: Option<&str>) {
    let clean = |v: Option<&str>, max| v.map(|s| report::slug(s, max, "-_.:/@"));
    if let Ok(mut ctx) = SESSION.lock() {
        ctx.provider = clean(provider, 40).filter(|s| !s.is_empty());
        ctx.model = clean(model, 80).filter(|s| !s.is_empty());
        ctx.temper = clean(temper, 24).filter(|s| !s.is_empty());
    }
}

/// Note a feature flag that is on (`lattice`, `mesh`...). At most 16, slugs only.
pub fn add_flag(flag: &str) {
    let flag = report::slug(flag, 40, "-_.");
    if flag.is_empty() {
        return;
    }
    if let Ok(mut ctx) = SESSION.lock() {
        if ctx.flags.len() < 16 && !ctx.flags.contains(&flag) {
            ctx.flags.push(flag);
        }
    }
}

pub(crate) fn context() -> Context {
    let host = HOST.get();
    let (provider, model, temper, flags) = match SESSION.try_lock() {
        Ok(ctx) => (
            ctx.provider.clone(),
            ctx.model.clone(),
            ctx.temper.clone(),
            ctx.flags.clone(),
        ),
        Err(_) => (None, None, None, Vec::new()),
    };
    Context {
        version: host
            .map(|h| h.version.clone())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string()),
        git_sha: host
            .and_then(|h| h.git_sha.clone())
            .map(|s| report::slug(&s, 12, ""))
            .filter(|s| s.len() >= 7)
            .unwrap_or_else(|| "unknown".to_string()),
        surface: host.map(|h| h.surface).unwrap_or(Surface::Cli),
        os: match std::env::consts::OS {
            "linux" | "macos" | "windows" | "ios" | "android" => std::env::consts::OS.to_string(),
            _ => "other".to_string(),
        },
        arch: std::env::consts::ARCH.to_string(),
        os_version: None,
        install_method: host
            .and_then(|h| h.install_method.clone())
            .map(|m| report::slug(&m, 24, "-_.")),
        provider,
        model,
        temper,
        flags,
    }
}
