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

struct ConfiguredMetadataReader {
    searches: Arc<AtomicUsize>,
    details: Arc<AtomicUsize>,
}

impl bluefin_discovery::catalog::metadata::MetadataReader for ConfiguredMetadataReader {
    fn search<'a>(
        &'a self,
        _kind: bluefin_contract::public_catalog::CatalogKind,
        term: &'a str,
        page: u32,
    ) -> bluefin_discovery::catalog::metadata::MetadataFuture<'a> {
        Box::pin(async move {
            use bluefin_discovery::{
                catalog::metadata::MetadataDocument,
                catalog_types::SourceStatus,
            };
            self.searches.fetch_add(1, Ordering::SeqCst);
            match term {
                "Missing" => return MetadataDocument::failure(SourceStatus::NotFound),
                "Limited" => return MetadataDocument::failure(SourceStatus::RateLimited),
                "Denied" | "The Matrix" => return MetadataDocument::failure(SourceStatus::Denied),
                _ => {}
            }
            MetadataDocument {
                body: Some(json!({
                    "page": if term == "Bad page" { page + 1 } else { page },
                    "total_pages": 3, "total_results": 41,
                    "results": [{
                        "id": 603, "title": "Fixture movie", "release_date": "1999-03-31",
                        "poster_path": "/poster.jpg", "backdrop_path": "/backdrop.jpg",
                        "genres": [{"name": "Drama"}],
                        "mediaInfo": {"owner": "PRIVATE_SHOULD_NOT_LEAK"}
                    }]
                })),
                observed_at: 42,
                status: SourceStatus::Complete,
            }
        })
    }

    fn details<'a>(
        &'a self,
        _identity: &'a bluefin_discovery::catalog_types::CatalogIdentity,
    ) -> bluefin_discovery::catalog::metadata::MetadataFuture<'a> {
        Box::pin(async move {
            self.details.fetch_add(1, Ordering::SeqCst);
            let cast: Vec<Value> = (1..=30).map(|id| json!({
                "id": id, "name": format!("Fixture person {id}"),
                "character": format!("Role {id}"), "profile_path": "/person.jpg"
            })).collect();
            bluefin_discovery::catalog::metadata::MetadataDocument {
                body: Some(json!({
                    "id": 603, "title": "Fixture movie",
                    "poster_path": "/poster.jpg", "backdrop_path": "/backdrop.jpg",
                    "genres": [{"name": "Drama"}], "credits": {"cast": cast, "crew": []},
                    "mediaInfo": {"owner": "PRIVATE_SHOULD_NOT_LEAK"}
                })),
                observed_at: 43,
                status: bluefin_discovery::catalog_types::SourceStatus::Complete,
            }
        })
    }
}

fn configured_reader() -> (Arc<ConfiguredMetadataReader>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let searches = Arc::new(AtomicUsize::new(0));
    let details = Arc::new(AtomicUsize::new(0));
    (Arc::new(ConfiguredMetadataReader {
        searches: Arc::clone(&searches),
        details: Arc::clone(&details),
    }), searches, details)
}

#[tokio::test]
async fn public_metadata_router_shares_one_reader_and_preserves_extended_records() {
    use bluefin_contract::public_catalog::{CatalogId, CatalogPage, CatalogRecord};
    use bluefin_discovery::catalog::CatalogService;
    let root = tempfile::tempdir().unwrap();
    let (reader, searches, details) = configured_reader();
    let catalog = Arc::new(CatalogService::open(&root.path().join("bluefin-catalog.json"))
        .unwrap().with_metadata_reader(reader));
    let discovery = DiscoveryService::open_with_catalog(
        &root.path().join("bluefin-discovery.json"), Some(Arc::clone(&catalog)),
    ).unwrap();
    assert!(Arc::ptr_eq(&catalog, discovery.catalog().unwrap()));
    let (origin, task) = serve(create_router(fake_state()
        .with_public_catalog(catalog).with_discovery(discovery))).await;
    let client = reqwest::Client::new();
    for _ in 0..2 {
        let response = client.get(format!(
            "{origin}/Bluefin/Catalog/Search?kind=movie&query=Fixture&page=2"
        )).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let page: CatalogPage = response.json().await.unwrap();
        assert_eq!(page.total_results, Some(41));
        assert_eq!(page.next_page, Some(3));
        assert_eq!(page.results[0].id, CatalogId::TmdbMovie(603));
        assert_eq!(page.results[0].genres, ["Drama"]);
        assert!(page.results[0].backdrop.is_some());
        assert_eq!(page.sources[0].observed_at, 42);
        assert!(!serde_json::to_string(&page).unwrap().contains("PRIVATE_SHOULD_NOT_LEAK"));
    }
    assert_eq!(searches.load(Ordering::SeqCst), 1);
    let response = client.get(format!("{origin}/Bluefin/Catalog/Details/movie/603"))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let record: CatalogRecord = response.json().await.unwrap();
    assert_eq!(record.credits.len(), 30);
    assert_eq!(record.credits[0].id, CatalogId::TmdbPerson(1));
    assert!(record.credits[0].artwork.is_some());
    assert!(!record.credits_truncated);
    assert_eq!(record.sources[0].observed_at, 43);
    assert_eq!(details.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn public_metadata_is_independent_of_private_discovery_but_bad_catalog_fails_closed() {
    use bluefin_discovery::catalog::CatalogService;
    let root = tempfile::tempdir().unwrap();
    let (reader, _, _) = configured_reader();
    let catalog = Arc::new(CatalogService::open(&root.path().join("bluefin-catalog.json"))
        .unwrap().with_metadata_reader(reader));
    let discovery_path = root.path().join("bluefin-discovery.json");
    std::fs::write(&discovery_path, "{").unwrap();
    assert!(DiscoveryService::open_with_catalog(&discovery_path, Some(Arc::clone(&catalog))).is_err());
    let (origin, task) = serve(create_router(fake_state().with_public_catalog(catalog))).await;
    let response = reqwest::get(format!(
        "{origin}/Bluefin/Catalog/Search?kind=movie&query=Fixture"
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    task.abort();

    let invalid_root = tempfile::tempdir().unwrap();
    let catalog_path = invalid_root.path().join("bluefin-catalog.json");
    std::fs::write(&catalog_path, "{").unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&catalog_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let discovery = DiscoveryService::open(&invalid_root.path().join("bluefin-discovery.json")).unwrap();
    assert!(discovery.catalog().is_none());
    let (origin, task) = serve(create_router(fake_state().with_discovery(discovery))).await;
    let response = reqwest::get(format!(
        "{origin}/Bluefin/Catalog/Search?kind=movie&query=Fixture"
    )).await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn injected_metadata_preserves_source_failures_and_rejects_wrong_identity_or_page() {
    use bluefin_discovery::catalog::CatalogService;
    let root = tempfile::tempdir().unwrap();
    let (reader, _, _) = configured_reader();
    let catalog = Arc::new(CatalogService::open(&root.path().join("bluefin-catalog.json"))
        .unwrap().with_metadata_reader(reader));
    let (origin, task) = serve(create_router(fake_state().with_public_catalog(catalog))).await;
    let client = reqwest::Client::new();
    for (query, expected, source_status) in [
        ("Missing", StatusCode::NOT_FOUND, "notFound"),
        ("Limited", StatusCode::TOO_MANY_REQUESTS, "rateLimited"),
        ("Bad%20page", StatusCode::BAD_GATEWAY, "invalidResponse"),
    ] {
        let response = client.get(format!(
            "{origin}/Bluefin/Catalog/Search?kind=movie&query={query}"
        )).send().await.unwrap();
        assert_eq!(response.status(), expected);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["sources"][0]["status"], source_status);
    }
    let response = client.get(format!("{origin}/Bluefin/Catalog/Details/movie/604"))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["sources"][0]["status"], "identityMismatch");
    task.abort();
}

#[tokio::test]
async fn injected_metadata_falls_back_to_authorized_service_but_not_after_not_found() {
    use bluefin_discovery::catalog::CatalogService;
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let (upstream, upstream_task) = serve(Router::new().route("/api/v1/search", get(
        move |headers: HeaderMap, uri: axum::extract::OriginalUri| {
            let count = Arc::clone(&count);
            async move {
                count.fetch_add(1, Ordering::SeqCst);
                metadata_search_fixture(headers, uri).await
            }
        }
    ))).await;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("bluefin-catalog.json");
    std::fs::write(&path, serde_json::to_vec(&json!({
        "seerrUrl": upstream, "seerrApiKey": "fixture-private-catalog-key",
        "allowLocalHttp": true, "disablePublicWatchPages": true
    })).unwrap()).unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let (reader, _, _) = configured_reader();
    let catalog = Arc::new(CatalogService::open(&path).unwrap().with_metadata_reader(reader));
    let (origin, task) = serve(create_router(fake_state().with_public_catalog(catalog))).await;
    let client = reqwest::Client::new();
    let response = client.get(format!(
        "{origin}/Bluefin/Catalog/Search?kind=movie&query=The%20Matrix&page=2"
    )).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page: Value = response.json().await.unwrap();
    assert_eq!(page["sources"][0]["status"], "denied");
    assert_eq!(page["sources"][1]["source"], "seerr");
    assert_eq!(page["results"][0]["id"]["value"], 603);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let response = client.get(format!(
        "{origin}/Bluefin/Catalog/Search?kind=movie&query=Missing"
    )).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    task.abort();
    upstream_task.abort();
}
