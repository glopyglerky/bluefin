//! Exercise discovery permission/body gates through the real HTTP router.
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use bluefin_discovery::DiscoveryService;
use ferrofin_api::{
    create_router,
    test_support::{authed_fake_state_with_policy, elevated_fake_state, fake_state},
};
use ferrofin_model::users::UserPolicy;
use serde_json::{Value, json};
use tower::ServiceExt;

#[tokio::test]
async fn anonymous_nonadmin_and_api_key_cannot_use_viewer_discovery_or_admin_configuration() {
    let root = tempfile::tempdir().unwrap();
    let service = DiscoveryService::open(&root.path().join("private.json")).unwrap();
    for (state, path, status) in [
        (
            fake_state(),
            "/Bluefin/Discovery/Search?query=original",
            StatusCode::UNAUTHORIZED,
        ),
        (
            authed_fake_state_with_policy(UserPolicy::default()),
            "/Bluefin/Admin/Discovery",
            StatusCode::FORBIDDEN,
        ),
        (
            elevated_fake_state(),
            "/Bluefin/Discovery/Search?query=original",
            StatusCode::FORBIDDEN,
        ),
    ] {
        let response = create_router(state.with_discovery(service.clone()))
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
    }
}

#[tokio::test]
async fn admin_write_is_cas_write_only_and_unknown_request_fields_cannot_elevate() {
    let root = tempfile::tempdir().unwrap();
    let service = DiscoveryService::open(&root.path().join("private.json")).unwrap();
    let app = create_router(elevated_fake_state().with_discovery(service.clone()));
    let update = json!({"expectedRevision":0,"enabled":false,"exposeToClients":false,
        "baseUrl":"https://requests.example.com","allowLocalHttp":false,"apiKey":"fixture-private-key","viewers":[]});
    for (expected, status) in [(0, StatusCode::OK), (0, StatusCode::CONFLICT)] {
        let mut body = update.clone();
        body["expectedRevision"] = json!(expected);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/Bluefin/Admin/Discovery")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        let bytes = to_bytes(response.into_body(), 16384).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("fixture-private-key"));
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/Bluefin/Admin/Discovery")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let snapshot: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
    assert_eq!(snapshot["credentialConfigured"], true);
    assert!(snapshot.get("apiKey").is_none());
    let user =
        create_router(authed_fake_state_with_policy(UserPolicy::default()).with_discovery(service));
    let response=user.clone().oneshot(Request::builder().uri("/Bluefin/Discovery/Search?query=original&userId=00000000000000000000000000000001").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response=user.oneshot(Request::builder().method("POST").uri("/Bluefin/Discovery/Requests").header("content-type","application/json").body(Body::from(json!({"actionId":"00000000-0000-0000-0000-000000000001","selections":[],"authorizedUseConfirmed":true,"userId":1,"ignoreQuota":true}).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
