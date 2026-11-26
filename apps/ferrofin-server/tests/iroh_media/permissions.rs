use super::*;
use futures_util::StreamExt as _;
use tokio::io::AsyncWriteExt as _;

async fn gated_upstream() -> (
    std::net::SocketAddr,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (release, wait) = tokio::sync::oneshot::channel::<()>();
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            request.push(socket.read_u8().await.unwrap());
            assert!(request.len() < 16 * 1024);
        }
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 8192\r\nConnection: close\r\n\r\n").await.unwrap();
        wait.await.unwrap();
        socket.write_all(&[0x5a; 8192]).await.unwrap();
        socket.shutdown().await.unwrap();
    });
    (address, release, upstream)
}

async fn delayed_remote_frame(revoke: bool) {
    let fixture = Fixture::new(512 * 1024).await;
    let (address, release, upstream) = gated_upstream().await;
    let id = uuid::Uuid::parse_str(&fixture.item).unwrap();
    let changed = sqlx::query("UPDATE BaseItems SET MediaType='Video',Type='MediaBrowser.Controller.Entities.Movies.Movie',Data=? WHERE Id=?")
        .bind(json!({"IsShortcut":true,"ShortcutPath":format!("http://{address}/synthetic.mp4"),"Container":"mp4"}).to_string())
        .bind(ferrofin_db::store::guid_to_db(id))
        .execute(fixture.db.pool()).await.unwrap();
    assert_eq!(changed.rows_affected(), 1);
    let sources = fixture
        .wired
        .state
        .media_sources
        .get_static_media_sources(id, false, None)
        .await
        .unwrap();
    assert_eq!(
        sources[0].protocol,
        ferrofin_model::media_info::MediaProtocol::Http
    );
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/Videos/{id}/stream?Static=true"))
                .header(header::AUTHORIZATION, auth(&fixture.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body().into_data_stream();
    let result = {
        let next = body.next();
        tokio::pin!(next);
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut next)
                .await
                .is_err(),
            "upstream has supplied headers, but no body bytes"
        );
        if revoke {
            let changed = sqlx::query("UPDATE Permissions SET Value=0 WHERE UserId=? AND Kind=?")
                .bind(ferrofin_db::store::guid_to_db(
                    uuid::Uuid::parse_str(&fixture.viewer).unwrap(),
                ))
                .bind(i32::from(
                    ferrofin_db::enums::PermissionKind::EnableMediaPlayback,
                ))
                .execute(fixture.db.pool())
                .await
                .unwrap();
            assert_eq!(changed.rows_affected(), 1);
        }
        let user = fixture
            .wired
            .state
            .users
            .get_user_by_id(uuid::Uuid::parse_str(&fixture.viewer).unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            fixture
                .wired
                .state
                .users
                .current_media_playback_allowed(&user)
                .await
                .unwrap(),
            !revoke
        );
        release.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), &mut next)
            .await
            .unwrap()
    };
    let delivered = result
        .as_ref()
        .and_then(|frame| frame.as_ref().ok())
        .map_or(0, axum::body::Bytes::len);
    eprintln!("delayed static body: revoked={revoke}, delivered_bytes={delivered}");
    drop(body);
    upstream.await.unwrap();
    fixture.close().await;
    if revoke {
        assert!(
            matches!(result, Some(Err(_))),
            "new bytes produced after verified revocation must be refused, delivered={delivered}"
        );
    } else {
        assert_eq!(result.unwrap().unwrap().as_ref(), &[0x5a; 8192]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn pending_remote_frame_is_refused_after_revocation() {
    delayed_remote_frame(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn pending_remote_frame_is_delivered_while_allowed() {
    delayed_remote_frame(false).await;
}
