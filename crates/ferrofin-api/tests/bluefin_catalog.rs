//! Public catalog reads work before pairing, preserve regions, and hide credentials.
use axum::{Json, Router, extract::State, http::{HeaderMap,StatusCode}, routing::get};
use bluefin_discovery::DiscoveryService;
use ferrofin_api::{create_router,test_support::fake_state};
use serde_json::{Value,json};
use std::sync::{Arc,atomic::{AtomicUsize,Ordering}};

async fn fixture(State(calls):State<Arc<AtomicUsize>>,headers:HeaderMap) -> Result<Json<Value>,StatusCode> {
    if headers.get("X-Api-Key").and_then(|value|value.to_str().ok()) != Some("fixture-private-catalog-key") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    calls.fetch_add(1,Ordering::SeqCst);
    Ok(Json(json!({"id":278,"watchProviders":[
        {"iso_3166_1":"US","link":"https://www.themoviedb.org/movie/278/watch?locale=US",
         "free":[{"id":999999,"name":"A new free service","logoPath":"/new.png"}]},
        {"iso_3166_1":"GB","link":"https://www.themoviedb.org/movie/278/watch?locale=GB",
         "buy":[{"id":10,"name":"Amazon Video","logoPath":"/amazon.png"}]}]})))
}
async fn serve(router:Router)->(String,tokio::task::JoinHandle<()>) {
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address=listener.local_addr().unwrap();
    let task=tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    (format!("http://{address}"),task)
}
#[tokio::test]
async fn anonymous_real_http_returns_dynamic_region_offers_and_coalesces_cache() {
    let calls=Arc::new(AtomicUsize::new(0));
    let (upstream,upstream_task)=serve(Router::new().route("/api/v1/movie/278",get(fixture))
        .with_state(Arc::clone(&calls))).await;
    let root=tempfile::tempdir().unwrap();
    let config=root.path().join("bluefin-catalog.json");
    let bytes=serde_json::to_vec(&json!({
        "seerrUrl":upstream,"seerrApiKey":"fixture-private-catalog-key","allowLocalHttp":true,
        "disablePublicWatchPages":true})).unwrap();
    std::fs::write(&config,bytes).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config,std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let service=DiscoveryService::open(&root.path().join("bluefin-discovery.json")).unwrap();
    let (origin,task)=serve(create_router(fake_state().with_discovery(service))).await;
    let client=reqwest::Client::new();
    let path="/Bluefin/Catalog/Offers?kind=movie&source=tmdb&id=movie%2F278&country=US";
    for _ in 0..2 {
        let response=client.get(format!("{origin}{path}")).send().await.unwrap();
        assert_eq!(response.status(),StatusCode::OK);
        let body=response.text().await.unwrap();
        assert!(!body.contains("fixture-private-catalog-key"));
        assert!(!body.contains("127.0.0.1"));
        let data:Value=serde_json::from_str(&body).unwrap();
        assert_eq!(data["country"],"US");
        assert_eq!(data["offers"].as_array().unwrap().len(),1);
        assert_eq!(data["offers"][0]["service"]["name"],"A new free service");
        assert_eq!(data["offers"][0]["accessType"],"free");
        assert_eq!(data["offers"][0]["destinations"],json!([]));
    }
    assert_eq!(calls.load(Ordering::SeqCst),1);
    let all:Value=client.post(format!("{origin}/Bluefin/Catalog/Offers")).json(&json!({
        "identity":{"kind":"movie","ids":[{"source":"tmdb","id":"movie/278"}]}
    })).send().await.unwrap().json().await.unwrap();
    assert_eq!(all["country"],Value::Null);
    assert_eq!(all["offers"].as_array().unwrap().len(),2);
    assert!(all["sources"].as_array().unwrap().iter().any(|s|s["source"]=="seerr" && s["status"]=="partial"));
    let response=client.get(format!("{origin}/Bluefin/Catalog/Offers?kind=movie&source=tmdb&id=278&country=USA"))
        .send().await.unwrap();
    assert_eq!(response.status(),StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst),2);
    task.abort(); upstream_task.abort();
}
