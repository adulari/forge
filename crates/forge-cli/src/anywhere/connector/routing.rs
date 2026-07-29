//! Explicit relay-command to local-daemon route policy.

use super::*;

pub(super) fn route_target(request: &BridgeRequest) -> Result<RouteTarget> {
    let exact = |method, path: &str| -> Result<RouteTarget> {
        Ok(RouteTarget {
            method,
            path: path.to_owned(),
            query: query_parameter(&request.parameters, 0)?,
        })
    };
    match request.route {
        RouteId::ListSessions => exact(Method::GET, "/api/sessions"),
        RouteId::CreateSession => exact(Method::POST, "/api/sessions"),
        RouteId::SessionHistory => exact(Method::GET, "/api/history"),
        RouteId::PastSessions => exact(Method::GET, "/api/sessions/past"),
        RouteId::SessionTree => exact(Method::GET, "/api/sessions/tree"),
        RouteId::ListProjects => exact(Method::GET, "/api/projects"),
        RouteId::BrowseProjects => exact(Method::GET, "/api/projects/browse"),
        RouteId::Upload => exact(Method::POST, "/api/upload"),
        RouteId::VoiceTranscribe => exact(Method::POST, "/api/voice/transcribe"),
        RouteId::ListSkills => exact(Method::GET, "/api/skills"),
        RouteId::ListModels => exact(Method::GET, "/api/models"),
        RouteId::ReadConfig => exact(Method::GET, "/api/config"),
        RouteId::UpdateConfig => exact(Method::PUT, "/api/config"),
        RouteId::ListHooks => exact(Method::GET, "/api/hooks"),
        RouteId::ListPlans => exact(Method::GET, "/api/plans"),
        RouteId::ReadMcp => exact(Method::GET, "/api/mcp"),
        RouteId::UpdateMcp => exact(Method::POST, "/api/mcp"),
        RouteId::Usage => exact(Method::GET, "/api/usage"),
        RouteId::Answer => exact(Method::POST, "/api/answer"),
        RouteId::PushKey => exact(Method::GET, "/api/push/key"),
        RouteId::PushSubscribe => exact(Method::POST, "/api/push/subscribe"),
        RouteId::PushUnsubscribe => exact(Method::POST, "/api/push/unsubscribe"),
        RouteId::ArchiveSession
        | RouteId::ForkSession
        | RouteId::MergeSession
        | RouteId::DiscardSession => {
            if request.parameters.is_empty() || request.parameters.len() > 2 {
                bail!("session route requires one path parameter and an optional query");
            }
            let id = safe_path_segment(&request.parameters[0])?;
            let operation = match request.route {
                RouteId::ArchiveSession => "archive",
                RouteId::ForkSession => "fork",
                RouteId::MergeSession => "merge",
                RouteId::DiscardSession => "discard",
                _ => unreachable!(),
            };
            Ok(RouteTarget {
                method: Method::POST,
                path: format!("/api/sessions/{id}/{operation}"),
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::Health | RouteId::SessionSnapshot | RouteId::SessionInput | RouteId::WebSocket => {
            bail!("route is not an HTTP bridge route")
        }
    }
}

pub(super) fn query_parameter(parameters: &[String], index: usize) -> Result<Option<String>> {
    if parameters.len() > index + 1 {
        bail!("bridge route contains unexpected parameters");
    }
    let Some(query) = parameters.get(index) else {
        return Ok(None);
    };
    if query.is_empty() {
        return Ok(None);
    }
    if query.len() > MAX_QUERY_LEN || !query.starts_with('?') || query.contains('#') {
        bail!("invalid bridge query parameter");
    }
    Ok(Some(query.trim_start_matches('?').to_owned()))
}

pub(super) fn safe_path_segment(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("invalid session id path parameter");
    }
    Ok(value)
}
