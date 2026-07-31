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
        RouteId::SearchSessions => exact(Method::GET, "/api/sessions/search"),
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
        RouteId::Diagnostics => exact(Method::GET, "/api/diagnostics"),
        RouteId::Answer => exact(Method::POST, "/api/answer"),
        RouteId::PushKey => exact(Method::GET, "/api/push/key"),
        RouteId::PushSubscribe => exact(Method::POST, "/api/push/subscribe"),
        RouteId::PushUnsubscribe => exact(Method::POST, "/api/push/unsubscribe"),
        RouteId::ListTerminals => exact(Method::GET, "/api/terminals"),
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
        RouteId::RenameSession | RouteId::DeleteSession => {
            if request.parameters.is_empty() || request.parameters.len() > 2 {
                bail!("session metadata route requires one path parameter and an optional query");
            }
            let id = safe_path_segment(&request.parameters[0])?;
            Ok(RouteTarget {
                method: if request.route == RouteId::RenameSession {
                    Method::PATCH
                } else {
                    Method::DELETE
                },
                path: format!("/api/sessions/{id}"),
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::Health
        | RouteId::SessionSnapshot
        | RouteId::SessionInput
        | RouteId::WebSocket
        | RouteId::TerminalWebSocket => bail!("route is not an HTTP bridge route"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anywhere::connector::command_journal::validate_command_request;

    fn request(route: RouteId, method: &str, parameters: &[&str]) -> BridgeRequest {
        BridgeRequest {
            request_id: [7; 16],
            route,
            method: method.to_owned(),
            parameters: parameters.iter().map(|value| (*value).to_owned()).collect(),
            headers: Vec::new(),
            body: Vec::new(),
            body_blob: None,
        }
    }

    #[test]
    fn session_search_and_metadata_routes_are_typed_and_method_checked() {
        let search = request(RouteId::SearchSessions, "GET", &["?q=needle&limit=30"]);
        let target = route_target(&search).unwrap();
        assert_eq!(target.method, Method::GET);
        assert_eq!(target.path, "/api/sessions/search");
        assert_eq!(target.query.as_deref(), Some("q=needle&limit=30"));
        assert!(validate_command_request(&search).is_ok());

        let rename = request(RouteId::RenameSession, "PATCH", &["session_7"]);
        let target = route_target(&rename).unwrap();
        assert_eq!(target.method, Method::PATCH);
        assert_eq!(target.path, "/api/sessions/session_7");
        assert!(validate_command_request(&rename).is_ok());

        let delete = request(RouteId::DeleteSession, "DELETE", &["session_7"]);
        let target = route_target(&delete).unwrap();
        assert_eq!(target.method, Method::DELETE);
        assert_eq!(target.path, "/api/sessions/session_7");
        assert!(validate_command_request(&delete).is_ok());

        assert!(validate_command_request(&request(
            RouteId::DeleteSession,
            "PATCH",
            &["session_7"],
        ))
        .is_err());
        assert!(route_target(&request(RouteId::RenameSession, "PATCH", &["unsafe/id"],)).is_err());
    }

    #[test]
    fn diagnostics_route_is_typed_and_method_checked() {
        let diagnostics = request(RouteId::Diagnostics, "GET", &[]);
        let target = route_target(&diagnostics).unwrap();
        assert_eq!(target.method, Method::GET);
        assert_eq!(target.path, "/api/diagnostics");
        assert_eq!(target.query, None);
        assert!(validate_command_request(&diagnostics).is_ok());

        assert!(validate_command_request(&request(RouteId::Diagnostics, "POST", &[])).is_err());
    }
}
