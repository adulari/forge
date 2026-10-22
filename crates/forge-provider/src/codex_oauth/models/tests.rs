use super::*;
use crate::EntitlementStatus;

/// The shape `GET /models` really returns, cut to the fields Forge reads: the account's listed
/// models plus the hidden internal entries the Codex CLI also carries.
const CATALOG: &str = r#"{"models":[
    {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","visibility":"list","priority":1},
    {"slug":"gpt-6-luna","display_name":"GPT-6-Luna","visibility":"list","priority":5},
    {"slug":"gpt-reserve","visibility":"hide"},
    {"slug":"gpt-5.6-luna","visibility":"list","priority":9},
    {"slug":"codex-auto-review","visibility":"hide"}
]}"#;

const MINI_REFUSAL: &str = r#"{"detail":"The 'gpt-5.4-mini' model is not supported when using Codex with a ChatGPT account."}"#;

fn ids(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| (*n).to_string()).collect()
}

fn provider_against(base: String) -> CodexOauthProvider {
    let tokens = forge_config::oauth::OAuthTokens {
        access_token: "at-acct".into(),
        refresh_token: Some("rt-acct".into()),
        // Never expires, so no refresh is attempted.
        expires_at: 0,
        token_endpoint: CODEX_OAUTH_TOKEN_ENDPOINT.into(),
        client_id: CODEX_OAUTH_CLIENT_ID.into(),
        scopes: vec![],
    };
    CodexOauthProvider::new()
        .with_api_base(base)
        .with_accounts(forge_config::oauth::OAuthAccountStore::new_single(
            "acct", tokens,
        ))
        .with_test_account_header("acct")
}

#[tokio::test]
async fn a_fresh_install_learns_the_models_from_the_backend() {
    let server = httpmock::MockServer::start();
    let listed = server.mock(|when, then| {
        when.method(httpmock::Method::GET)
            .path("/models")
            .query_param("client_version", CATALOG_CLIENT_VERSION)
            .header("authorization", "Bearer at-acct")
            .header("chatgpt-account-id", "acct");
        then.status(200)
            .header("content-type", "application/json")
            .body(CATALOG);
    });

    let models = provider_against(server.base_url())
        .fetch_catalog()
        .await
        .expect("catalog");

    assert_eq!(
        models,
        ids(&["gpt-6.1-sol", "gpt-6-luna", "gpt-5.6-luna"]),
        "listed models in the backend's order; hidden internal entries never appear"
    );
    assert_eq!(listed.calls(), 1);
}

#[tokio::test]
async fn a_model_the_seeds_never_heard_of_is_offered_when_the_backend_lists_it() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        then.status(200)
            .body(r#"{"models":[{"slug":"gpt-7-nova","visibility":"list"}]}"#);
    });

    let models = provider_against(server.base_url())
        .fetch_catalog()
        .await
        .expect("catalog");

    assert!(!SEED_MODELS.contains(&"gpt-7-nova"));
    assert_eq!(
        namespaced(&models),
        ["codex-oauth::gpt-7-nova"],
        "no release is needed to see a model the backend ships"
    );
}

#[tokio::test]
async fn a_rejected_catalog_request_is_an_auth_error_not_an_empty_list() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        then.status(403).body(r#"{"detail":"not entitled"}"#);
    });

    let error = provider_against(server.base_url())
        .fetch_catalog()
        .await
        .expect_err("403 is a failure");

    assert!(error.is_auth(), "{error}");
}

#[tokio::test]
async fn an_empty_catalog_is_a_failed_lookup_so_the_fallback_applies() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        // What a client version the backend considers too old receives.
        then.status(200).body(r#"{"models":[]}"#);
    });

    let error = provider_against(server.base_url())
        .fetch_catalog()
        .await
        .expect_err("no models is not an answer");

    assert!(matches!(error, ProviderError::Unavailable(_)), "{error}");
}

#[test]
fn the_seeds_never_carry_a_model_a_chatgpt_account_is_refused() {
    assert!(!SEED_MODELS.contains(&"gpt-5.4-mini"));
    for id in [
        "gpt-6.1-sol",
        "gpt-6-sol",
        "gpt-6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
    ] {
        assert!(
            SEED_MODELS.contains(&id),
            "{id} missing from the seeds: {SEED_MODELS:?}"
        );
    }
}

#[test]
fn seed_models_are_namespaced_chat_models() {
    let seeds = namespaced(&ids(SEED_MODELS));
    assert_eq!(seeds.len(), SEED_MODELS.len());
    for id in &seeds {
        assert!(id.starts_with("codex-oauth::"));
        assert!(!forge_config::is_non_chat_model(id), "{id}");
    }
}

#[test]
fn the_probe_model_comes_from_what_the_account_can_run() {
    let listed = ids(&["gpt-6.1-sol", "gpt-6-luna", "gpt-6-sol"]);
    assert_eq!(
        probe_model(&listed),
        "gpt-6-luna",
        "the small tier is enough"
    );

    let no_small_tier = ids(&["gpt-6.1-sol", "gpt-6-sol"]);
    assert_eq!(probe_model(&no_small_tier), "gpt-6.1-sol");
}

#[test]
fn the_probe_never_picks_a_websocket_only_model() {
    // `gpt-5.6-luna` 404s over the plain HTTPS the probe uses, which would read as a dead login.
    assert!(codex_websocket::CODEX_WEBSOCKET_MODELS.contains(&"gpt-5.6-luna"));
    let listed = ids(&["gpt-5.6-luna", "gpt-5.6-sol"]);
    assert_eq!(probe_model(&listed), "gpt-5.6-sol");
}

#[test]
fn with_no_catalog_the_probe_still_picks_a_model_that_works() {
    let model = probe_model(&[]);
    assert!(SEED_MODELS.contains(&model.as_str()), "{model}");
    assert_ne!(model, "gpt-5.4-mini");
    assert!(!codex_websocket::CODEX_WEBSOCKET_MODELS.contains(&model.as_str()));
}

/// The defect: sign-in succeeded, then the entitlement probe asked for `gpt-5.4-mini`, which the
/// backend refuses for ChatGPT accounts, so the user was shown a 400 about a model they never chose.
#[tokio::test]
async fn sign_in_probe_uses_a_model_the_account_can_run() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        then.status(200).body(CATALOG);
    });
    let refused_mini = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/responses")
            .json_body_includes(r#"{"model":"gpt-5.4-mini"}"#);
        then.status(400)
            .header("content-type", "application/json")
            .body(MINI_REFUSAL);
    });
    let accepted = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/responses")
            .json_body_includes(r#"{"model":"gpt-6-luna","stream":true,"store":false}"#)
            .header("chatgpt-account-id", "acct");
        then.status(200)
            .header("content-type", "text/event-stream")
            .body("data: {}\n\n");
    });

    let (status, listed) = probe_entitlement_at(&server.base_url(), "at-acct", "acct")
        .await
        .expect("probe");

    assert_eq!(status, EntitlementStatus::Entitled);
    assert_eq!(listed, ids(&["gpt-6.1-sol", "gpt-6-luna", "gpt-5.6-luna"]));
    assert_eq!(refused_mini.calls(), 0, "the refused model is never asked");
    assert_eq!(accepted.calls(), 1);
}

#[tokio::test]
async fn sign_in_probe_survives_an_unreachable_catalog() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        then.status(500).body("upstream error");
    });
    let refused_mini = server.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .path("/responses")
            .json_body_includes(r#"{"model":"gpt-5.4-mini"}"#);
        then.status(400).body(MINI_REFUSAL);
    });
    let accepted = server.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/responses");
        then.status(200).body("data: {}\n\n");
    });

    let (status, listed) = probe_entitlement_at(&server.base_url(), "at-acct", "acct")
        .await
        .expect("probe");

    assert_eq!(status, EntitlementStatus::Entitled);
    assert!(
        listed.is_empty(),
        "nothing to remember from a failed lookup"
    );
    assert_eq!(refused_mini.calls(), 0);
    assert_eq!(accepted.calls(), 1);
}

#[tokio::test]
async fn a_dead_login_is_still_reported_by_the_probe() {
    let server = httpmock::MockServer::start();
    server.mock(|when, then| {
        when.method(httpmock::Method::GET).path("/models");
        then.status(401).body("{}");
    });
    server.mock(|when, then| {
        when.method(httpmock::Method::POST).path("/responses");
        then.status(401).body(r#"{"detail":"token rejected"}"#);
    });

    let (status, _) = probe_entitlement_at(&server.base_url(), "at-acct", "acct")
        .await
        .expect("probe");

    assert!(
        matches!(status, EntitlementStatus::AuthFailed(_)),
        "{status:?}"
    );
}

fn cache_with(entries: &[(&str, &[&str])]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bridge-models.json");
    for (key, models) in entries {
        remember_bridge_models_at(&path, key, &ids(models), 1_000);
    }
    (dir, path)
}

#[test]
fn the_last_list_the_backend_gave_beats_everything_else() {
    let (_dir, path) = cache_with(&[
        (CACHE_KEY, &["gpt-7-nova"]),
        ("codex-cli", &["gpt-6-luna", "gpt-6-sol"]),
    ]);
    assert_eq!(fallback_from(Some(&path), true), ids(&["gpt-7-nova"]));
}

#[test]
fn the_cli_list_stands_in_only_for_a_chatgpt_login() {
    let (_dir, path) = cache_with(&[("codex-cli", &["gpt-6-nova", "gpt-5.4-mini"])]);

    assert_eq!(
        fallback_from(Some(&path), true),
        ids(&["gpt-6-nova", "gpt-5.4-mini"]),
        "the same account sees the same models"
    );
    assert_eq!(
        fallback_from(Some(&path), false),
        SEED_MODELS
            .iter()
            .map(|m| (*m).to_string())
            .collect::<Vec<_>>(),
        "an API-key CLI advertises models a ChatGPT login is refused"
    );
}

#[test]
fn with_nothing_remembered_the_seeds_are_the_floor() {
    let seeds: Vec<String> = SEED_MODELS.iter().map(|m| (*m).to_string()).collect();
    assert_eq!(
        fallback_from(
            Some(std::path::Path::new(
                "/nonexistent/forge/bridge-models.json"
            )),
            true
        ),
        seeds
    );
    assert_eq!(fallback_from(None, true), seeds);
    let (_dir, path) = cache_with(&[("claude-cli", &["opus"])]);
    assert_eq!(
        fallback_from(Some(&path), true),
        seeds,
        "another bridge's list is not this provider's"
    );
}

#[test]
fn the_remembered_list_round_trips_through_the_shared_cache() {
    let (_dir, path) = cache_with(&[("codex-cli", &["gpt-6-luna"])]);
    remember_bridge_models_at(&path, CACHE_KEY, &ids(&["gpt-7-nova"]), 2_000);

    let cache = read_bridge_model_cache(&path);
    assert_eq!(cache[CACHE_KEY].models, ids(&["gpt-7-nova"]));
    assert_eq!(
        cache["codex-cli"].models,
        ids(&["gpt-6-luna"]),
        "remembering one provider leaves the others alone"
    );
}

#[test]
fn model_ids_are_namespaced_once_and_cleaned() {
    assert_eq!(
        namespaced(&ids(&["gpt-6-luna", " gpt-6-sol ", "", "gpt-6-luna"])),
        ["codex-oauth::gpt-6-luna", "codex-oauth::gpt-6-sol"]
    );
}
