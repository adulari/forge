//! Static PWA asset handlers for `forge serve`.

use super::*;

pub(super) async fn page(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [
            (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (axum::http::header::X_FRAME_OPTIONS, "DENY"),
            (
                axum::http::header::CONTENT_SECURITY_POLICY,
                remote::PAGE_CSP,
            ),
            (axum::http::header::REFERRER_POLICY, "no-referrer"),
        ],
        remote::CONTROL_PAGE.replace("__BASE__", &state.base),
    )
        .into_response()
}

pub(super) async fn app_js(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        remote::APP_JS.replace("__BASE__", &state.base),
    )
        .into_response()
}

pub(super) async fn styles_css() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css")],
        remote::STYLES_CSS,
    )
        .into_response()
}

pub(super) async fn manifest(State(state): State<Arc<DaemonState>>) -> Response {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/manifest+json",
        )],
        remote::manifest_json(&state.base),
    )
        .into_response()
}

pub(super) async fn service_worker() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "text/javascript")],
        remote::SERVICE_WORKER,
    )
        .into_response()
}

pub(super) async fn icon() -> Response {
    (
        [(axum::http::header::CONTENT_TYPE, "image/svg+xml")],
        remote::ICON_SVG,
    )
        .into_response()
}
