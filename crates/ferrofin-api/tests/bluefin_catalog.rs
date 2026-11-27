//! Public catalog reads work before pairing, preserve regions, and hide credentials.
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::get,
};
use bluefin_discovery::DiscoveryService;
use ferrofin_api::{create_router, test_support::fake_state};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

async fn fixture(
    State(calls): State<Arc<AtomicUsize>>,
    headers: HeaderMap,
) -> Result<Json<Value>, StatusCode> {
    if headers
        .get("X-Api-Key")
        .and_then(|value| value.to_str().ok())
        != Some("fixture-private-catalog-key")
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    calls.fetch_add(1, Ordering::SeqCst);
    Ok(Json(json!({"id":278,"watchProviders":[
        {"iso_3166_1":"US","link":"https://www.themoviedb.org/movie/278/watch?locale=US",
         "free":[{"id":999999,"name":"A new free service","logoPath":"/new.png"}]},
        {"iso_3166_1":"GB","link":"https://www.themoviedb.org/movie/278/watch?locale=GB",
         "buy":[{"id":10,"name":"Amazon Video","logoPath":"/amazon.png"}]}]})))
}
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}"), task)
}
#[tokio::test]
async fn anonymous_real_http_returns_dynamic_region_offers_and_coalesces_cache() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (upstream, upstream_task) = serve(
        Router::new()
            .route("/api/v1/movie/278", get(fixture))
            .with_state(Arc::clone(&calls)),
    )
    .await;
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("bluefin-catalog.json");
    let bytes = serde_json::to_vec(&json!({
        "seerrUrl":upstream,"seerrApiKey":"fixture-private-catalog-key","allowLocalHttp":true,
        "disablePublicWatchPages":true}))
    .unwrap();
    std::fs::write(&config, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let service = DiscoveryService::open(&root.path().join("bluefin-discovery.json")).unwrap();
    let (origin, task) = serve(create_router(fake_state().with_discovery(service))).await;
    let client = reqwest::Client::new();
    let path = "/Bluefin/Catalog/Offers?kind=movie&source=tmdb&id=movie%2F278&country=US";
    for _ in 0..2 {
        let response = client.get(format!("{origin}{path}")).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(!body.contains("fixture-private-catalog-key"));
        assert!(!body.contains("127.0.0.1"));
        let data: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(data["country"], "US");
        assert_eq!(data["offers"].as_array().unwrap().len(), 1);
        assert_eq!(data["offers"][0]["service"]["name"], "A new free service");
        assert_eq!(data["offers"][0]["accessType"], "free");
        assert_eq!(data["offers"][0]["destinations"], json!([]));
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let all: Value = client
        .post(format!("{origin}/Bluefin/Catalog/Offers"))
        .json(&json!({
            "identity":{"kind":"movie","ids":[{"source":"tmdb","id":"movie/278"}]}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all["country"], Value::Null);
    assert_eq!(all["offers"].as_array().unwrap().len(), 2);
    assert!(
        all["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerr" && s["status"] == "partial")
    );
    let response = client
        .get(format!(
            "{origin}/Bluefin/Catalog/Offers?kind=movie&source=tmdb&id=278&country=USA"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    task.abort();
    upstream_task.abort();
}

async fn metadata_search_fixture(
    headers: HeaderMap,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
) -> Result<Json<Value>, StatusCode> {
    assert_eq!(
        headers
            .get("X-Api-Key")
            .and_then(|value| value.to_str().ok()),
        Some("fixture-private-catalog-key")
    );
    let query = uri.query().unwrap_or_default();
    if query.contains("OverLimit") {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    assert!(query.contains("query=The%20Matrix"), "{query}");
    assert!(query.contains("page=2"), "{query}");
    assert!(!query.contains("The+Matrix"));
    Ok(Json(
        json!({"page": 2, "totalPages": 3, "totalResults": 60, "results": [
            {"id": 603, "mediaType": "movie", "title": "The Matrix", "releaseDate": "1999-03-31",
             "posterPath": "/poster.jpg", "overview": "Public synopsis",
             "mediaInfo": {"requests": [{"requestedBy": "PRIVATE_SHOULD_NOT_LEAK"}]},
             "onUserWatchlist": true},
            {"id": 1396, "mediaType": "tv", "name": "Breaking Bad"},
            {"id": 6384, "mediaType": "person", "name": "Keanu Reeves"}
        ]}),
    ))
}
async fn metadata_movie_fixture() -> Json<Value> {
    Json(
        json!({"id":603,"title":"The Matrix","releaseDate":"1999-03-31",
        "posterPath":"/poster.jpg","backdropPath":"/backdrop.jpg","overview":"Public synopsis",
        "credits":{"cast":[{"id":6384,"name":"Keanu Reeves","character":"Neo","profilePath":"/profile.jpg"}],"crew":[]},
        "watchProviders":[],"mediaInfo":{"requests":["PRIVATE_SHOULD_NOT_LEAK"]}}),
    )
}
async fn metadata_person_fixture() -> Json<Value> {
    Json(
        json!({"id":6384,"name":"Keanu Reeves","biography":"Public biography",
        "profilePath":"/profile.jpg","imdbId":"nm0000206"}),
    )
}
async fn metadata_person_credits_fixture() -> Json<Value> {
    let mut cast: Vec<Value> = (1..=30).map(|id|json!({
        "id":id,"mediaType":"movie","title":format!("Fixture film {id}"),
        "character":format!("Fixture role {id}"),"releaseDate":"2000-01-01","posterPath":"/film.jpg"
    })).collect();
    cast.push(
        json!({"id":1396,"mediaType":"tv","name":"Fixture series","character":"Fixture role",
        "firstAirDate":"2008-01-20","posterPath":"/series.jpg"}),
    );
    Json(json!({"id":6384,"cast":cast,"crew":[],"mediaInfo":{"owner":"PRIVATE_SHOULD_NOT_LEAK"}}))
}

#[tokio::test]
async fn anonymous_metadata_uses_authorized_service_with_typed_art_credits_and_explicit_failures() {
    let upstream_router = Router::new()
        .route("/api/v1/search", get(metadata_search_fixture))
        .route("/api/v1/movie/603", get(metadata_movie_fixture))
        .route("/api/v1/person/6384", get(metadata_person_fixture))
        .route(
            "/api/v1/person/6384/combined_credits",
            get(metadata_person_credits_fixture),
        )
        .route(
            "/api/v1/person/2",
            get(|| async { Json(json!({"id":2,"name":"Fixture person"})) }),
        )
        .route(
            "/api/v1/person/2/combined_credits",
            get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        );
    let (upstream, upstream_task) = serve(upstream_router).await;
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("bluefin-catalog.json");
    std::fs::write(
        &config,
        serde_json::to_vec(&json!({
            "seerrUrl":upstream,"seerrApiKey":"fixture-private-catalog-key",
            "allowLocalHttp":true,"disablePublicWatchPages":true
        }))
        .unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let service = DiscoveryService::open(&root.path().join("bluefin-discovery.json")).unwrap();
    let (origin, task) = serve(create_router(fake_state().with_discovery(service))).await;
    let client = reqwest::Client::new();
    let response = client
        .get(format!(
            "{origin}/Bluefin/Catalog/Search?kind=movie&query=The%20Matrix&page=2"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    for private in [
        "PRIVATE_SHOULD_NOT_LEAK",
        "mediaInfo",
        "onUserWatchlist",
        "fixture-private-catalog-key",
        "127.0.0.1",
    ] {
        assert!(!body.contains(private), "public metadata leaked {private}");
    }
    let search: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(search["results"].as_array().unwrap().len(), 1);
    assert_eq!(
        search["results"][0]["id"],
        json!({"kind":"tmdbMovie","value":603})
    );
    assert_eq!(
        search["results"][0]["artwork"]["url"],
        "https://image.tmdb.org/t/p/w342/poster.jpg"
    );
    assert_eq!(search["nextPage"], 3);
    assert_eq!(search["totalResults"], Value::Null);
    assert!(
        search["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerr" && s["status"] == "complete")
    );

    let response = client
        .get(format!("{origin}/Bluefin/Catalog/Details/movie/603"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let movie: Value = response.json().await.unwrap();
    assert_eq!(
        movie["credits"][0]["id"],
        json!({"kind":"tmdbPerson","value":6384})
    );
    assert_eq!(movie["credits"][0]["role"], "Neo");
    assert_eq!(
        movie["backdrop"]["url"],
        "https://image.tmdb.org/t/p/w780/backdrop.jpg"
    );
    assert!(movie.get("mediaInfo").is_none());

    let response = client
        .get(format!("{origin}/Bluefin/Catalog/Details/person/6384"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let person: Value = response.json().await.unwrap();
    assert_eq!(person["credits"].as_array().unwrap().len(), 31);
    assert_eq!(
        person["credits"][30]["id"],
        json!({"kind":"tmdbSeries","value":1396})
    );
    assert_eq!(person["credits"][30]["year"], 2008);
    assert_eq!(person["creditsTruncated"], false);

    let partial: Value = client
        .get(format!("{origin}/Bluefin/Catalog/Details/person/2"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        partial["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerr" && s["status"] == "partial")
    );
    assert!(
        partial["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerrCredits" && s["status"] == "unavailable")
    );
    let rate = client
        .get(format!(
            "{origin}/Bluefin/Catalog/Search?kind=movie&query=OverLimit"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(rate.status(), StatusCode::TOO_MANY_REQUESTS);
    let failure: Value = rate.json().await.unwrap();
    assert!(failure.get("results").is_none());
    assert!(
        failure["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerr" && s["status"] == "rateLimited")
    );
    let invalid = client
        .get(format!(
            "{origin}/Bluefin/Catalog/Search?kind=movie&query=x"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);

    // A missing cross-reference is incomplete evidence, not proof of a different title.
    let offers:Value=client.post(format!("{origin}/Bluefin/Catalog/Offers")).json(&json!({
        "identity":{"kind":"movie","ids":[{"source":"tmdb","id":"movie/603"},{"source":"imdb","id":"tt0133093"}]},
        "country":"US"
    })).send().await.unwrap().json().await.unwrap();
    assert!(
        offers["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["source"] == "seerr" && s["status"] == "invalidResponse")
    );
    assert!(
        !offers["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["status"] == "identityMismatch")
    );
    assert!(offers["offers"].as_array().unwrap().is_empty());
    task.abort();
    upstream_task.abort();
}
