//! Best-effort metadata-only notification requests.

use reqwest::Client;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GenericPushEvent {
    AttentionRequired,
    JobCompleted,
    JobFailed,
    WorkspaceReady,
}

#[derive(Serialize)]
struct NotificationRequest<'a> {
    event: GenericPushEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_device_id: Option<&'a str>,
}

/// Request a generic notification without allowing notification failure to fail the core action.
pub(crate) async fn request_best_effort(
    http: &Client,
    service_url: &str,
    access_token: &str,
    target_device_id: Option<&str>,
    event: GenericPushEvent,
    idempotency_key: &str,
) {
    let result = http
        .post(format!("{service_url}/v1/push/notifications"))
        .bearer_auth(access_token)
        .header("Idempotency-Key", idempotency_key)
        .json(&NotificationRequest {
            event,
            target_device_id,
        })
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {}
        Ok(response) => eprintln!(
            "⚠ Generic Forge Anywhere notification unavailable (HTTP {}); core work succeeded.",
            response.status().as_u16()
        ),
        Err(_) => {
            eprintln!("⚠ Generic Forge Anywhere notification unavailable; core work succeeded.")
        }
    }
}

/// Anywhere leg of the `push_notification` tool. The hosted service only accepts a closed set of
/// content-free events, so the phone gets the generic `attention_required` alert and the real
/// text stays on the host (`GET /api/notices`, read over the end-to-end encrypted bridge).
/// `None` when this host is not enrolled and enabled.
pub(crate) async fn tool_hint() -> Option<Result<(), String>> {
    let config = forge_config::load().ok()?;
    if !config.anywhere.enabled {
        return None;
    }
    let store = super::StateStore::platform().ok()?;
    let mut state = store.load().ok()?;
    let outcome = async {
        let token = super::ensure_access_token(&store, &mut state)
            .await
            .map_err(|e| format!("not signed in: {e}"))?;
        let http = super::client().map_err(|e| e.to_string())?;
        let response = http
            .post(format!(
                "{}/v1/push/notifications",
                config.anywhere.service_url()
            ))
            .bearer_auth(token)
            .header("Idempotency-Key", super::idempotency_key())
            .json(&NotificationRequest {
                event: GenericPushEvent::AttentionRequired,
                target_device_id: None,
            })
            .send()
            .await
            .map_err(|_| "service unreachable".to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "service answered HTTP {}",
                response.status().as_u16()
            ))
        }
    }
    .await;
    Some(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_request_has_only_fixed_event_and_routing_id() {
        let request = NotificationRequest {
            event: GenericPushEvent::WorkspaceReady,
            target_device_id: Some("11"),
        };
        let json = serde_json::to_value(request).expect("serialize request");
        assert_eq!(json["event"], "workspace_ready");
        assert_eq!(json["target_device_id"], "11");
        assert_eq!(json.as_object().expect("object").len(), 2);
    }

    #[test]
    fn all_notification_categories_are_closed_and_generic() {
        let encoded = [
            GenericPushEvent::AttentionRequired,
            GenericPushEvent::JobCompleted,
            GenericPushEvent::JobFailed,
            GenericPushEvent::WorkspaceReady,
        ]
        .map(|event| serde_json::to_string(&event).expect("serialize event"));
        assert_eq!(
            encoded,
            [
                "\"attention_required\"",
                "\"job_completed\"",
                "\"job_failed\"",
                "\"workspace_ready\"",
            ]
        );
    }
}
