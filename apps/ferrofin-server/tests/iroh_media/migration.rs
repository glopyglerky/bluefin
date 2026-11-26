use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn changed_socket_during_handshake_never_reaches_media_admission() {
    let f = Fixture::new(512 * 1024).await;
    let mut server = f.start().await;
    let ep = endpoint(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)).await;
    let front = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
    let via = front.local_addr().unwrap();
    let first = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let second = tokio::net::UdpSocket::bind(SocketAddr::new(nonloopback(), 0))
        .await
        .unwrap();
    let destination = server.address;
    let (stop, mut stopped) = watch::channel(false);
    let moved = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let packets = moved.clone();
    let returned = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let replies = returned.clone();
    let bridge = tokio::spawn(async move {
        let mut inbound = vec![0; 65536];
        let mut original = vec![0; 65536];
        let mut replacement = vec![0; 65536];
        let mut client = None;
        let mut switched = false;
        loop {
            tokio::select! {biased;
                ()=async {let _=stopped.wait_for(|v|*v).await;}=>break,
                result=front.recv_from(&mut inbound)=>{
                    let(n,addr)=result.unwrap();client=Some(addr);
                    if switched {packets.fetch_add(1,std::sync::atomic::Ordering::SeqCst);second.send_to(&inbound[..n],destination).await.unwrap();}
                    else {first.send_to(&inbound[..n],destination).await.unwrap();}
                },
                result=first.recv_from(&mut original)=>{
                    let(n,addr)=result.unwrap();assert_eq!(addr,destination);
                    if !switched && let Some(client)=client {
                        // Switch before exposing the first server flight: every
                        // client Handshake packet now has the changed source.
                        switched=true;front.send_to(&original[..n],client).await.unwrap();
                    }
                },
                result=second.recv_from(&mut replacement)=>{
                    let(n,addr)=result.unwrap();assert_eq!(addr,destination);
                    replies.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                    if let Some(client)=client {front.send_to(&replacement[..n],client).await.unwrap();}
                },
            }
        }
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let result = tokio::time::timeout_at(
        deadline,
        ep.connect(EndpointAddr::new(server.id).with_ip_addr(via), ALPN),
    )
    .await;
    if let Ok(Ok(conn)) = result {
        // The client's TLS completion can precede receipt of its authenticated
        // Handshake flight by the server. Test real media admission, not that
        // local client's completion notification.
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(raw("GET", &f.path(), &auth(&f.token), "").as_bytes())
            .await
            .unwrap();
        send.finish().unwrap();
        let response = tokio::time::timeout_at(deadline, recv.read_to_end(16 * 1024)).await;
        assert!(
            !matches!(response, Ok(Ok(ref bytes)) if !bytes.is_empty()),
            "changed-address handshake must not deliver application bytes"
        );
        conn.close(0u32.into(), b"local cancellation of unadmitted handshake");
    }
    assert!(moved.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(returned.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(server.seen.lock().unwrap().is_empty());
    tokio::time::timeout(Duration::from_secs(5), server.active.wait_for(|n| *n == 0))
        .await
        .unwrap()
        .unwrap();
    let receipt = tokio::time::timeout(Duration::from_secs(5), server.stop())
        .await
        .unwrap();
    assert!(receipt.endpoint_closed);
    stop.send_replace(true);
    bridge.await.unwrap();
    ep.close().await;
    f.close().await;
}

struct Bridge {
    via: SocketAddr,
    initial: SocketAddr,
    replacement: SocketAddr,
    switch: watch::Sender<bool>,
    stop: watch::Sender<bool>,
    moved: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    returned: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    job: tokio::task::JoinHandle<()>,
}
impl Bridge {
    async fn start(destination: SocketAddr, initial_ip: IpAddr, replacement_ip: IpAddr) -> Self {
        let front = tokio::net::UdpSocket::bind("[::1]:0").await.unwrap();
        let via = front.local_addr().unwrap();
        let first = tokio::net::UdpSocket::bind(SocketAddr::new(initial_ip, 0))
            .await
            .unwrap();
        let initial = first.local_addr().unwrap();
        let second = tokio::net::UdpSocket::bind(SocketAddr::new(replacement_ip, 0))
            .await
            .unwrap();
        let replacement = second.local_addr().unwrap();
        let (switch, changed) = watch::channel(false);
        let (stop, mut stopped) = watch::channel(false);
        let moved = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let packets = moved.clone();
        let returned = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let replies = returned.clone();
        let job = tokio::spawn(async move {
            let mut inbound = vec![0; 65536];
            let mut original_reply = vec![0; 65536];
            let mut replacement_reply = vec![0; 65536];
            let mut client = None;
            loop {
                tokio::select! {biased;
                  ()=async {let _=stopped.wait_for(|v|*v).await;}=>break,
                  result=front.recv_from(&mut inbound)=>{
                    let(n,addr)=result.unwrap();client=Some(addr);
                    if *changed.borrow() {packets.fetch_add(1,std::sync::atomic::Ordering::SeqCst);second.send_to(&inbound[..n],destination).await.unwrap();}
                    else {first.send_to(&inbound[..n],destination).await.unwrap();}
                  },
                  result=first.recv_from(&mut original_reply)=>{let(n,addr)=result.unwrap();assert_eq!(addr,destination);if !*changed.borrow() && let Some(client)=client {front.send_to(&original_reply[..n],client).await.unwrap();}},
                  result=second.recv_from(&mut replacement_reply)=>{let(n,addr)=result.unwrap();assert_eq!(addr,destination);replies.fetch_add(1,std::sync::atomic::Ordering::SeqCst);if let Some(client)=client {front.send_to(&replacement_reply[..n],client).await.unwrap();}},
                }
            }
        });
        Self {
            via,
            initial,
            replacement,
            switch,
            stop,
            moved,
            returned,
            job,
        }
    }
    async fn stop(self) {
        self.stop.send_replace(true);
        self.job.await.unwrap();
    }
}
async fn observe_rebinding(
    conn: &iroh::endpoint::Connection,
    recv: &mut iroh::endpoint::RecvStream,
    bridge: &Bridge,
    direction: &str,
    sample: usize,
) {
    // Positive control drains more than the unchanged 8 MiB receive window.
    let mut control = 0usize;
    let mut buf = vec![0; 32 * 1024];
    tokio::time::timeout(Duration::from_secs(5), async {
        while control <= 8 * 1024 * 1024 {
            control += recv.read(&mut buf).await.unwrap().unwrap();
        }
    })
    .await
    .unwrap();
    bridge.switch.send_replace(true);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut drained = 0usize;
    let closed=tokio::time::timeout_at(deadline,async {
      loop {tokio::select! {biased;
        closed=conn.closed()=>break closed,
        read=recv.read(&mut buf),if drained<16*1024*1024=>match read {Ok(Some(n))=>drained+=n,_=>return conn.closed().await,},
      }}
    }).await;
    let packets = bridge.moved.load(std::sync::atomic::Ordering::SeqCst);
    let replies = bridge.returned.load(std::sync::atomic::Ordering::SeqCst);
    eprintln!(
        "fixed-path {direction} sample={sample} positive_control_bytes={control} replacement_packets={packets} replacement_replies={replies} post_switch_buffered_bytes={drained} actual_client_closure={} (deadline expiry is not closure)",
        closed.is_ok()
    );
    assert!(
        packets > 0,
        "actual encrypted packets crossed the replacement source socket"
    );
    assert_eq!(
        replies, 0,
        "no server output was redirected to the replacement tuple"
    );
    assert!(
        drained <= 8 * 1024 * 1024,
        "changed-address frames must not advance past pre-switch receive credit: {drained}"
    );
}
async fn reconnect(f: &Fixture, server: &Running, ep: &Endpoint, bridge: &Bridge) {
    let fresh = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(EndpointAddr::new(server.id).with_ip_addr(bridge.via), ALPN),
    )
    .await
    .unwrap()
    .unwrap();
    let (mut send, mut recv) = fresh.open_bi().await.unwrap();
    send.write_all(raw("GET", &f.path(), &auth(&f.token), "Range: bytes=0-1023\r\n").as_bytes())
        .await
        .unwrap();
    send.finish().unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 206 "));
    assert_eq!(
        recv.read_to_end(1024).await.unwrap(),
        std::fs::read(f.temp.path().join("music/Original.wav")).unwrap()[..1024]
    );
    assert_eq!(
        server.seen.lock().unwrap().last().unwrap().remote,
        bridge.replacement
    );
    fresh.close(0u32.into(), b"fresh application drained");
}
async fn rebinding_sample(
    f: &Fixture,
    direction: &str,
    initial_ip: IpAddr,
    replacement_ip: IpAddr,
    sample: usize,
) {
    let server = f.start().await;
    let address = server.address;
    let ep = endpoint(IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)).await;
    let bridge = Bridge::start(address, initial_ip, replacement_ip).await;
    let conn = tokio::time::timeout(
        Duration::from_secs(5),
        ep.connect(EndpointAddr::new(server.id).with_ip_addr(bridge.via), ALPN),
    )
    .await
    .unwrap()
    .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(raw("GET", &f.path(), &auth(&f.token), "").as_bytes())
        .await
        .unwrap();
    send.finish().unwrap();
    assert!(read_head(&mut recv).await.starts_with("HTTP/1.1 200 "));
    assert_eq!(
        server.seen.lock().unwrap().last().unwrap().remote,
        bridge.initial
    );
    observe_rebinding(&conn, &mut recv, &bridge, direction, sample).await;
    assert_eq!(server.seen.lock().unwrap().len(), 1);
    // NEW same-identity handshake over the changed socket receives fresh admission.
    reconnect(f, &server, &ep, &bridge).await;
    conn.close(0u32.into(), b"local cancellation after rejected rebinding");
    let active = server.active.clone();
    let receipt = tokio::time::timeout(Duration::from_secs(5), server.stop())
        .await
        .unwrap();
    assert!(receipt.endpoint_closed && receipt.joined_connections >= 2);
    assert_eq!(
        *active.borrow(),
        0,
        "all connection/body/permit owners retired"
    );
    bridge.stop().await;
    drop((recv, send, conn));
    ep.close().await;
    // Complete every fixture owner before checking actual socket reuse.
    drop(std::net::UdpSocket::bind(address).unwrap());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
async fn real_udp_fixed_path_rebinding_rejects_progress_and_reconnects() {
    let f = Fixture::new(64 * 1024 * 1024).await;
    // Three fixed samples per direction, each once; family separation prevents
    // candidate exchange from bypassing the actual encrypted UDP bridge.
    for (direction, initial, replacement) in [
        (
            "loopback-to-nonloopback",
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            nonloopback(),
        ),
        (
            "nonloopback-to-loopback",
            nonloopback(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        ),
    ] {
        for sample in 0..3 {
            rebinding_sample(&f, direction, initial, replacement, sample).await;
        }
    }
    f.close().await;
}
