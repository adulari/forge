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
        RouteId::ModelReport => exact(Method::GET, "/api/models/report"),
        RouteId::ReadConfig => exact(Method::GET, "/api/config"),
        RouteId::UpdateConfig => exact(Method::PUT, "/api/config"),
        RouteId::ListHooks => exact(Method::GET, "/api/hooks"),
        RouteId::ListPlans => exact(Method::GET, "/api/plans"),
        // Read-only MCP review. Registering a server is not bridged: `POST /api/mcp` persists a
        // caller-supplied command that the host executes the next time a session starts that
        // client, which is more durable code execution than the commit the git routes below
        // already refuse. Configuring an MCP server stays a local action, on the machine that
        // runs it.
        RouteId::ReadMcp => exact(Method::GET, "/api/mcp"),
        RouteId::UpdateMcp => bail!("MCP server registration is not reachable over the bridge"),
        RouteId::Usage => exact(Method::GET, "/api/usage"),
        RouteId::Diagnostics => exact(Method::GET, "/api/diagnostics"),
        RouteId::Answer => exact(Method::POST, "/api/answer"),
        RouteId::PushKey => exact(Method::GET, "/api/push/key"),
        RouteId::PushSubscribe => exact(Method::POST, "/api/push/subscribe"),
        RouteId::PushUnsubscribe => exact(Method::POST, "/api/push/unsubscribe"),
        RouteId::ListTerminals => exact(Method::GET, "/api/terminals"),
        // Read-only git review (see the note on `RouteId::GitStatus`). `GET` only: the daemon's
        // mutating git endpoints stay unreachable over the bridge, so a compromised relay command
        // cannot stage or commit in the user's repository, any more than it can register an MCP
        // server the host would then run.
        RouteId::GitStatus => exact(Method::GET, "/api/git/status"),
        RouteId::GitBranches => exact(Method::GET, "/api/git/branches"),
        RouteId::GitDiff => exact(Method::GET, "/api/git/diff"),
        RouteId::Blame => exact(Method::GET, "/api/blame"),
        RouteId::ProjectChanges => exact(Method::GET, "/api/projects/changes"),
        RouteId::GitHunk => exact(Method::POST, "/api/git/hunk"),
        RouteId::WorkspaceEntries => exact(Method::GET, "/api/workspace/entries"),
        RouteId::ReadWorkspaceFile => exact(Method::GET, "/api/workspace/file"),
        RouteId::WriteWorkspaceFile => exact(Method::PUT, "/api/workspace/file"),
        RouteId::WorkspaceSearch => exact(Method::GET, "/api/workspace/search"),
        RouteId::ListWorkflows => exact(Method::GET, "/api/workflows"),
        RouteId::ListSchedules => exact(Method::GET, "/api/schedules"),
        RouteId::Changelog => exact(Method::GET, "/api/changelog"),
        RouteId::ListNotices => exact(Method::GET, "/api/notices"),
        RouteId::Brief => exact(Method::GET, "/api/brief"),
        RouteId::Identity => exact(Method::GET, "/api/identity"),
        RouteId::AccountSyncStatus => exact(Method::GET, "/api/anywhere/sync"),
        RouteId::AccountSyncUpdate => exact(Method::PUT, "/api/anywhere/sync"),
        RouteId::AccountSyncNow => exact(Method::POST, "/api/anywhere/sync/now"),
        RouteId::ImportScan => exact(Method::GET, "/api/import/scan"),
        RouteId::ListIssues => exact(Method::GET, "/api/issues"),
        RouteId::IssueDetail => exact(Method::GET, "/api/issues/detail"),
        RouteId::StartIssue => exact(Method::POST, "/api/issues/start"),
        RouteId::DrainIssues => exact(Method::POST, "/api/issues/drain"),
        RouteId::ListIssueDrains => exact(Method::GET, "/api/issues/drains"),
        RouteId::CancelIssueDrain => exact(Method::POST, "/api/issues/drain/cancel"),
        RouteId::SessionClaude | RouteId::ExportSessionClaude | RouteId::ContinueClaudeSession => {
            let id = single_path_parameter(request)?;
            let (method, path) = match request.route {
                RouteId::SessionClaude => (Method::GET, format!("/api/sessions/{id}/claude")),
                RouteId::ExportSessionClaude => {
                    (Method::POST, format!("/api/sessions/{id}/claude"))
                }
                _ => (Method::POST, format!("/api/claude/sessions/{id}/continue")),
            };
            Ok(RouteTarget {
                method,
                path,
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::SessionJobTail => {
            if request.parameters.len() < 2 || request.parameters.len() > 3 {
                bail!("job tail route requires a session id, a job id and an optional query");
            }
            let id = safe_path_segment(&request.parameters[0])?;
            let job: u32 = request.parameters[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid job id path parameter"))?;
            Ok(RouteTarget {
                method: Method::GET,
                path: format!("/api/sessions/{id}/jobs/{job}/tail"),
                query: query_parameter(&request.parameters, 2)?,
            })
        }
        RouteId::SessionDiff | RouteId::SessionAgents | RouteId::SessionJobs => {
            let id = single_path_parameter(request)?;
            let leaf = match request.route {
                RouteId::SessionDiff => "diff",
                RouteId::SessionAgents => "agents",
                _ => "jobs",
            };
            Ok(RouteTarget {
                method: Method::GET,
                path: format!("/api/sessions/{id}/{leaf}"),
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::PauseSchedule | RouteId::ResumeSchedule | RouteId::DeleteSchedule => {
            let id = single_path_parameter(request)?;
            let operation = match request.route {
                RouteId::PauseSchedule => "pause",
                RouteId::ResumeSchedule => "resume",
                _ => "delete",
            };
            Ok(RouteTarget {
                method: Method::POST,
                path: format!("/api/schedules/{id}/{operation}"),
                query: query_parameter(&request.parameters, 1)?,
            })
        }
        RouteId::ArchiveSession
        | RouteId::ForkSession
        | RouteId::MergeSession
        | RouteId::DiscardSession
        | RouteId::SetSessionMode
        | RouteId::SessionMessage => {
            if request.parameters.is_empty() || request.parameters.len() > 2 {
                bail!("session route requires one path parameter and an optional query");
            }
            let id = safe_path_segment(&request.parameters[0])?;
            let operation = match request.route {
                RouteId::ArchiveSession => "archive",
                RouteId::ForkSession => "fork",
                RouteId::MergeSession => "merge",
                RouteId::DiscardSession => "discard",
                RouteId::SetSessionMode => "mode",
                RouteId::SessionMessage => "message",
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

fn single_path_parameter(request: &BridgeRequest) -> Result<&str> {
    if request.parameters.is_empty() || request.parameters.len() > 2 {
        bail!("route requires one path parameter and an optional query");
    }
    safe_path_segment(&request.parameters[0])
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
    fn session_message_route_is_a_typed_post_to_one_session() {
        let message = request(RouteId::SessionMessage, "POST", &["session_7"]);
        let target = route_target(&message).unwrap();
        assert_eq!(target.method, Method::POST);
        assert_eq!(target.path, "/api/sessions/session_7/message");
        assert!(validate_command_request(&message).is_ok());
        assert!(
            validate_command_request(&request(RouteId::SessionMessage, "GET", &["s"])).is_err()
        );
        assert!(route_target(&request(RouteId::SessionMessage, "POST", &["../x"])).is_err());
        assert!(route_target(&request(RouteId::SessionMessage, "POST", &[])).is_err());
    }

    /// The three git routes read; nothing else about git is reachable over the bridge. A relay
    /// command asking for a mutating method on them must be refused, so a phone (or anything that
    /// can speak to the relay) can review a diff but never stage, commit or switch branches.
    #[test]
    fn git_review_routes_are_read_only_over_the_bridge() {
        let status = request(RouteId::GitStatus, "GET", &["?session=abc"]);
        let target = route_target(&status).unwrap();
        assert_eq!(target.method, Method::GET);
        assert_eq!(target.path, "/api/git/status");
        assert_eq!(target.query.as_deref(), Some("session=abc"));
        assert!(validate_command_request(&status).is_ok());

        let diff = request(RouteId::GitDiff, "GET", &["?session=abc&path=src/main.rs"]);
        let target = route_target(&diff).unwrap();
        assert_eq!(target.path, "/api/git/diff");
        assert!(validate_command_request(&diff).is_ok());

        let branches = request(RouteId::GitBranches, "GET", &[]);
        assert_eq!(route_target(&branches).unwrap().path, "/api/git/branches");
        assert!(validate_command_request(&branches).is_ok());

        for route in [RouteId::GitStatus, RouteId::GitBranches, RouteId::GitDiff] {
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                assert!(
                    validate_command_request(&request(route, method, &[])).is_err(),
                    "{route:?} must refuse {method} over the bridge"
                );
            }
        }
    }

    /// Hunk actions are the one git write the bridge carries; the read routes stay read-only and
    /// the hunk route accepts nothing but POST.
    #[test]
    fn only_the_hunk_route_writes_git_over_the_bridge() {
        let hunk = request(RouteId::GitHunk, "POST", &[]);
        assert!(validate_command_request(&hunk).is_ok());
        for method in ["GET", "PUT", "PATCH", "DELETE"] {
            assert!(validate_command_request(&request(RouteId::GitHunk, method, &[])).is_err());
        }
        for (route, parameters) in [
            (RouteId::Blame, &[][..]),
            (RouteId::ProjectChanges, &[][..]),
            (RouteId::SessionJobs, &["s"][..]),
        ] {
            for method in ["POST", "PUT", "PATCH", "DELETE"] {
                assert!(
                    validate_command_request(&request(route, method, parameters)).is_err(),
                    "{route:?} must refuse {method}"
                );
            }
        }
    }

    #[test]
    fn issue_routes_are_typed_and_only_start_writes() {
        for (route, method, path) in [
            (RouteId::ListIssues, "GET", "/api/issues"),
            (RouteId::IssueDetail, "GET", "/api/issues/detail"),
            (RouteId::StartIssue, "POST", "/api/issues/start"),
            (RouteId::DrainIssues, "POST", "/api/issues/drain"),
            (RouteId::ListIssueDrains, "GET", "/api/issues/drains"),
            (
                RouteId::CancelIssueDrain,
                "POST",
                "/api/issues/drain/cancel",
            ),
        ] {
            let target = route_target(&request(route, method, &[])).unwrap();
            assert_eq!(target.path, path);
            assert!(validate_command_request(&request(route, method, &[])).is_ok());
            let wrong = if method == "GET" { "POST" } else { "GET" };
            assert!(validate_command_request(&request(route, wrong, &[])).is_err());
            assert!(validate_command_request(&request(route, "DELETE", &[])).is_err());
        }
    }

    #[test]
    fn claude_hand_off_routes_are_typed_and_the_writes_are_post_only() {
        let cases = [
            (RouteId::ImportScan, "GET", &[][..], "/api/import/scan"),
            (
                RouteId::SessionClaude,
                "GET",
                &["s_1"][..],
                "/api/sessions/s_1/claude",
            ),
            (
                RouteId::ExportSessionClaude,
                "POST",
                &["s_1"][..],
                "/api/sessions/s_1/claude",
            ),
            (
                RouteId::ContinueClaudeSession,
                "POST",
                &["0a1b-22"][..],
                "/api/claude/sessions/0a1b-22/continue",
            ),
        ];
        for (route, method, parameters, path) in cases {
            let target = route_target(&request(route, method, parameters)).unwrap();
            assert_eq!(target.path, path);
            assert!(validate_command_request(&request(route, method, parameters)).is_ok());
        }
        for route in [RouteId::ExportSessionClaude, RouteId::ContinueClaudeSession] {
            assert!(validate_command_request(&request(route, "GET", &["s_1"])).is_err());
        }
        assert!(
            validate_command_request(&request(RouteId::SessionClaude, "POST", &["s_1"])).is_err()
        );
        assert!(route_target(&request(RouteId::ContinueClaudeSession, "POST", &["../x"])).is_err());
    }

    #[test]
    fn files_diff_and_schedule_routes_map_to_their_daemon_endpoints() {
        let cases: [(RouteId, &str, &[&str], Method, &str); 24] = [
            (
                RouteId::Blame,
                "GET",
                &["?session=s1&path=src/a.rs"],
                Method::GET,
                "/api/blame",
            ),
            (
                RouteId::ProjectChanges,
                "GET",
                &["?cwd=/home/me/proj"],
                Method::GET,
                "/api/projects/changes",
            ),
            (RouteId::GitHunk, "POST", &[], Method::POST, "/api/git/hunk"),
            (
                RouteId::SessionJobs,
                "GET",
                &["session_7"],
                Method::GET,
                "/api/sessions/session_7/jobs",
            ),
            (
                RouteId::SessionJobTail,
                "GET",
                &["session_7", "4242", "?lines=50"],
                Method::GET,
                "/api/sessions/session_7/jobs/4242/tail",
            ),
            (
                RouteId::WorkspaceEntries,
                "GET",
                &["?session=s1&path=src"],
                Method::GET,
                "/api/workspace/entries",
            ),
            (
                RouteId::ReadWorkspaceFile,
                "GET",
                &["?session=s1&path=a.rs"],
                Method::GET,
                "/api/workspace/file",
            ),
            (
                RouteId::WriteWorkspaceFile,
                "PUT",
                &[],
                Method::PUT,
                "/api/workspace/file",
            ),
            (
                RouteId::WorkspaceSearch,
                "GET",
                &["?session=s1&q=main"],
                Method::GET,
                "/api/workspace/search",
            ),
            (
                RouteId::ListWorkflows,
                "GET",
                &["?session=s1"],
                Method::GET,
                "/api/workflows",
            ),
            (
                RouteId::ListSchedules,
                "GET",
                &[],
                Method::GET,
                "/api/schedules",
            ),
            (
                RouteId::Changelog,
                "GET",
                &["?limit=5"],
                Method::GET,
                "/api/changelog",
            ),
            (
                RouteId::Brief,
                "GET",
                &["?since=100"],
                Method::GET,
                "/api/brief",
            ),
            (
                RouteId::ModelReport,
                "GET",
                &["?days=7"],
                Method::GET,
                "/api/models/report",
            ),
            (RouteId::Identity, "GET", &[], Method::GET, "/api/identity"),
            (
                RouteId::AccountSyncStatus,
                "GET",
                &[],
                Method::GET,
                "/api/anywhere/sync",
            ),
            (
                RouteId::AccountSyncUpdate,
                "PUT",
                &[],
                Method::PUT,
                "/api/anywhere/sync",
            ),
            (
                RouteId::AccountSyncNow,
                "POST",
                &[],
                Method::POST,
                "/api/anywhere/sync/now",
            ),
            (
                RouteId::ListNotices,
                "GET",
                &["?after=3"],
                Method::GET,
                "/api/notices",
            ),
            (
                RouteId::SessionDiff,
                "GET",
                &["session_7"],
                Method::GET,
                "/api/sessions/session_7/diff",
            ),
            (
                RouteId::SessionAgents,
                "GET",
                &["session_7"],
                Method::GET,
                "/api/sessions/session_7/agents",
            ),
            (
                RouteId::PauseSchedule,
                "POST",
                &["nightly-1"],
                Method::POST,
                "/api/schedules/nightly-1/pause",
            ),
            (
                RouteId::ResumeSchedule,
                "POST",
                &["nightly-1"],
                Method::POST,
                "/api/schedules/nightly-1/resume",
            ),
            (
                RouteId::DeleteSchedule,
                "POST",
                &["nightly-1"],
                Method::POST,
                "/api/schedules/nightly-1/delete",
            ),
        ];
        for (route, method, parameters, expected_method, expected_path) in cases {
            let bridged = request(route, method, parameters);
            let target = route_target(&bridged).unwrap();
            assert_eq!(target.method, expected_method, "{route:?}");
            assert_eq!(target.path, expected_path, "{route:?}");
            assert!(validate_command_request(&bridged).is_ok(), "{route:?}");
            assert!(
                validate_command_request(&request(route, "DELETE", parameters)).is_err(),
                "{route:?} must refuse a method it does not serve"
            );
        }
        assert!(route_target(&request(RouteId::SessionDiff, "GET", &["../x"])).is_err());
        assert!(route_target(&request(RouteId::SessionJobTail, "GET", &["s", "../x"])).is_err());
        assert!(route_target(&request(RouteId::SessionJobTail, "GET", &["s"])).is_err());
        assert!(route_target(&request(RouteId::SessionJobs, "GET", &["a/b"])).is_err());
        assert!(route_target(&request(RouteId::PauseSchedule, "POST", &[])).is_err());
    }

    /// The MCP catalog reads; registering a server does not. `POST /api/mcp` writes a
    /// caller-supplied command into `mcp.toml` that the host later executes, so bridging it would
    /// hand anything that can speak to the relay durable code execution — strictly more than the
    /// commit the git routes already refuse.
    #[test]
    fn mcp_registration_is_not_reachable_over_the_bridge() {
        let read = request(RouteId::ReadMcp, "GET", &[]);
        let target = route_target(&read).unwrap();
        assert_eq!(target.method, Method::GET);
        assert_eq!(target.path, "/api/mcp");
        assert!(validate_command_request(&read).is_ok());

        assert!(route_target(&request(RouteId::UpdateMcp, "POST", &[])).is_err());
        for method in ["POST", "PUT", "PATCH", "DELETE", "GET"] {
            assert!(
                validate_command_request(&request(RouteId::UpdateMcp, method, &[])).is_err(),
                "MCP registration must refuse {method} over the bridge"
            );
            if method != "GET" {
                assert!(
                    validate_command_request(&request(RouteId::ReadMcp, method, &[])).is_err(),
                    "the MCP catalog route must refuse {method} over the bridge"
                );
            }
        }
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
