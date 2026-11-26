//! Account-owned Home over the real server, authentication, HTTP and database.
use ferrofin_db::{Database, store::guid_to_db};
use ferrofin_server::config::Config;
use reqwest::{Client, Method, StatusCode};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

#[path = "support/boot.rs"]
mod boot;

const PASSWORD: &str = "synthetic-home-http-only";
const IDENTITY: &str =
    r#"Client="Bluefin Home test", Device="Fixture", DeviceId="home-http", Version="1""#;

async fn call(
    client: &Client,
    base: &str,
    method: Method,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let authorization = token.map_or_else(
        || format!("MediaBrowser {IDENTITY}"),
        |token| format!(r#"MediaBrowser {IDENTITY}, Token="{token}""#),
    );
    let mut request = client
        .request(method, format!("{base}{path}"))
        .header("Authorization", authorization);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().await.expect("fixture HTTP request");
    let status = response.status();
    let bytes = response.bytes().await.expect("fixture response body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn login(client: &Client, base: &str, name: &str) -> String {
    let (status, body) = call(
        client,
        base,
        Method::POST,
        "/Users/AuthenticateByName",
        None,
        Some(json!({"Username":name,"Pw":PASSWORD})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "synthetic account authenticates");
    body["AccessToken"]
        .as_str()
        .expect("fixture session token")
        .to_owned()
}
fn event(term: &str, generation: i64) -> Value {
    json!({"eventId":Uuid::new_v4(),"query":term,"generation":generation,
           "occurredAtUnixMs":chrono::Utc::now().timestamp_millis()})
}
async fn count(db: &Database, user: &str) -> i64 {
    sqlx::query_scalar(r#"SELECT COUNT(*) FROM "BluefinHomeSearches" WHERE "UserId" = ?"#)
        .bind(guid_to_db(Uuid::parse_str(user).unwrap()))
        .fetch_one(db.pool())
        .await
        .unwrap()
}

/// Use the same stored rows a scan produces, with distinct presentation keys and
/// owning library IDs. All writes go to this test's newly created database.
async fn seed_catalog(db: &Database) -> (Uuid, Uuid, Uuid, Uuid) {
    let allowed = Uuid::new_v4();
    let hidden = Uuid::new_v4();
    let visible_item = Uuid::new_v4();
    let hidden_item = Uuid::new_v4();
    for (id, folder, name, parent) in [
        (allowed, true, "Allowed", None),
        (hidden, true, "Hidden", None),
        (visible_item, false, "Ocean Documentary", Some(allowed)),
        (hidden_item, false, "Ocean Secret", Some(hidden)),
    ] {
        sqlx::query(r#"INSERT INTO "BaseItems"
            ("Id","Type","IsFolder","IsInMixedFolder","IsLocked","IsMovie","IsRepeat","IsSeries",
             "IsVirtualItem","Name","SortName","CleanName","ParentId","TopParentId","PresentationUniqueKey")
            VALUES (?1,?2,?3,0,0,?4,0,0,0,?5,?5,?6,?7,?7,?8)"#)
            .bind(guid_to_db(id))
            .bind(if folder { "MediaBrowser.Controller.Entities.CollectionFolder" }
                  else { "MediaBrowser.Controller.Entities.Movies.Movie" })
            .bind(folder).bind(!folder).bind(name).bind(name.to_lowercase())
            .bind(parent.map(guid_to_db)).bind(id.simple().to_string())
            .execute(db.writer()).await.expect("seed synthetic catalog");
    }
    (allowed, hidden, visible_item, hidden_item)
}

#[tokio::test(flavor = "multi_thread")]
async fn home_http_keeps_accounts_separate_and_never_resurrects_cleared_history() {
    let root = tempfile::tempdir().unwrap();
    for directory in ["data", "config", "cache"] {
        std::fs::create_dir_all(root.path().join(directory)).unwrap();
    }
    let config = Config {
        server_name: "bluefin-home-http".into(),
        admin_user: "home-admin".into(),
        admin_password: PASSWORD.into(),
        disable_extensions: true,
        detect_network_change: false,
        ..Config::test_stub(root.path())
    };
    let db = Database::connect(&config.database_url()).await.unwrap();
    db.run_migrations().await.unwrap();
    let (allowed, _, visible, hidden) = seed_catalog(&db).await;
    let client = Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let (server, base, _) = boot::spawn_server(&client, |port| Config {
        port,
        ..config.clone()
    })
    .await;
    for (method, path, body) in [
        (Method::GET, "/Bluefin/Home/Recommendations", None),
        (
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(event("ocean", 0)),
        ),
        (
            Method::DELETE,
            "/Bluefin/Home/Searches",
            Some(json!({"generation":0})),
        ),
    ] {
        assert_eq!(
            call(&client, &base, method, path, None, body).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    let admin = login(&client, &base, "home-admin").await;
    let mut users = Vec::new();
    for name in ["home-a", "home-b"] {
        let (status, created) = call(
            &client,
            &base,
            Method::POST,
            "/Users/New",
            Some(&admin),
            Some(json!({"Name":name,"Password":PASSWORD})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        users.push(created["Id"].as_str().unwrap().to_owned());
    }
    let (status, user) = call(
        &client,
        &base,
        Method::GET,
        &format!("/Users/{}", users[0]),
        Some(&admin),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut policy = user["Policy"].clone();
    policy["EnableAllFolders"] = json!(false);
    policy["EnabledFolders"] = json!([allowed]);
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            &format!("/Users/{}/Policy", users[0]),
            Some(&admin),
            Some(policy)
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let a = login(&client, &base, "home-a").await;
    let b = login(&client, &base, "home-b").await;
    for token in [&a, &b] {
        let (status, home) = call(
            &client,
            &base,
            Method::GET,
            "/Bluefin/Home/Recommendations",
            Some(token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            home["recommendations"],
            json!([]),
            "cold-start never invents preferences"
        );
        assert_eq!(home["searchHistoryGeneration"], 0);
    }
    let first = event("ocean", 0);
    for _ in 0..2 {
        assert_eq!(
            call(
                &client,
                &base,
                Method::POST,
                "/Bluefin/Home/Searches",
                Some(&a),
                Some(first.clone())
            )
            .await
            .0,
            StatusCode::OK
        );
    }
    assert_eq!(
        count(&db, &users[0]).await,
        1,
        "exact event retry is idempotent"
    );
    assert_eq!(
        count(&db, &users[1]).await,
        0,
        "another account has no inherited history"
    );
    let mut changed = first.clone();
    changed["query"] = json!("changed");
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(changed)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let (status, home) = call(
        &client,
        &base,
        Method::GET,
        "/Bluefin/Home/Recommendations",
        Some(&a),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<Uuid> = home["recommendations"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|shelf| shelf["items"].as_array().unwrap())
        .map(|item| Uuid::parse_str(item["Id"].as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![visible],
        "search shelves contain only accessible real catalog items"
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::GET,
            &format!("/Items/{hidden}"),
            Some(&admin),
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::GET,
            &format!("/Items/{hidden}"),
            Some(&a),
            None
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let mut hidden_selection = event("ocean", 0);
    hidden_selection["selectedItemId"] = json!(hidden);
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(hidden_selection)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        count(&db, &users[0]).await,
        1,
        "inaccessible selections never become history"
    );
    let mut spoof = event("piano", 0);
    spoof["userId"] = json!(users[1]);
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(spoof)
        )
        .await
        .0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            &format!("/Bluefin/Home/Searches?userId={}", users[1]),
            Some(&a),
            Some(event("piano", 0))
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Auth/Keys?app=home-http",
            Some(&admin),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, keys) = call(
        &client,
        &base,
        Method::GET,
        "/Auth/Keys",
        Some(&admin),
        None,
    )
    .await;
    let key = keys["Items"][0]["AccessToken"].as_str().unwrap();
    assert_eq!(
        call(
            &client,
            &base,
            Method::GET,
            "/Bluefin/Home/Recommendations",
            Some(key),
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&b),
            Some(event("piano", 0))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::DELETE,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(json!({"generation":0}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(first)
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(count(&db, &users[0]).await, 0);
    assert_eq!(
        count(&db, &users[1]).await,
        1,
        "one account cannot clear another account"
    );
    let fresh = event("ocean", 1);
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(fresh)
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::DELETE,
            "/Bluefin/Home/Searches",
            Some(&a),
            Some(json!({"generation":0}))
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        count(&db, &users[0]).await,
        1,
        "late clear retry retains searches submitted after clearing"
    );
    assert_eq!(
        call(
            &client,
            &base,
            Method::POST,
            "/System/Shutdown",
            Some(&admin),
            None
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    server
        .join()
        .expect("fixture server thread")
        .expect("clean fixture shutdown");
    db.close().await;
}
