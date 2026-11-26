use super::*;
use sha2::{Digest, Sha256};
use tokio::net::UdpSocket;

// An opaque UDP forwarder exercises QUIC recovery through real loopback sockets.
// It holds at most one datagram, flushes it within 2 ms, and deterministically
// drops/reorders packets after the handshake. It cannot bypass TLS or peer pins.
async fn impaired_path(
    server: SocketAddr,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    Arc<[AtomicUsize; 2]>,
) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    let counts = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let observed = counts.clone();
    let task = tokio::spawn(async move {
        let mut client = None;
        let mut packet = [0; 65536];
        let mut held: Option<(Vec<u8>, SocketAddr)> = None;
        let mut received = 0;
        let mut flush = tokio::time::interval(Duration::from_millis(2));
        loop {
            tokio::select! {
                _ = flush.tick(), if held.is_some() => {
                    let (bytes, target) = held.take().unwrap();
                    socket.send_to(&bytes, target).await.unwrap();
                }
                result = socket.recv_from(&mut packet) => {
                    let (len, source) = result.unwrap();
                    let target = if source == server {
                        client.expect("server traffic follows a client packet")
                    } else {
                        assert!(source.ip().is_loopback());
                        assert!(client.is_none_or(|previous| previous == source));
                        client = Some(source);
                        server
                    };
                    received += 1;
                    if received > 64 && received % 97 == 0 {
                        observed[0].fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if received > 64 && received % 11 == 0 && held.is_none() {
                        held = Some((packet[..len].to_vec(), target));
                        continue;
                    }
                    socket.send_to(&packet[..len], target).await.unwrap();
                    if let Some((bytes, target)) = held.take() {
                        socket.send_to(&bytes, target).await.unwrap();
                        observed[1].fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    });
    (address, task, counts)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_64m_streams_survive_bounded_stalls_loss_and_reordering() {
    // Each case runs once. A failure ends the test, with no retries.
    const TOTAL: usize = 64 * 1024 * 1024;
    let block: Vec<u8> = (0..65536)
        .map(|n| ((n * 31 + n / 257) % 251) as u8)
        .collect();
    let mut expected = Sha256::new();
    for _ in 0..TOTAL / block.len() {
        expected.update(&block);
    }
    let expected = expected.finalize();
    assert_eq!(
        format!("{expected:x}"),
        "0dd6cc27111d0a919a50510291ca3db78f7cef38bad6c2604da24f232a6601dd"
    );
    for sample in 0..3 {
        let server = endpoint().await.unwrap();
        let peer = server.id();
        let (address, forwarder, counts) = impaired_path(server.bound_sockets()[0]).await;
        let server_block = block.clone();
        let serve = tokio::spawn(async move {
            let connection = server.accept().await.unwrap().await.unwrap();
            let (mut send, mut recv) = connection.accept_bi().await.unwrap();
            assert_eq!(recv.read_to_end(1).await.unwrap(), [sample as u8]);
            let sizes: &[usize] = match sample {
                0 => &[32768],
                1 => &[257, 8192, 1200, 65536, 31],
                _ => &[65536, 127, 16384, 1419],
            };
            let mut at = 0;
            let mut index = 0;
            while at < TOTAL {
                let size = sizes[index % sizes.len()]
                    .min(server_block.len() - at % server_block.len())
                    .min(TOTAL - at);
                send.write_all(
                    &server_block[at % server_block.len()..at % server_block.len() + size],
                )
                .await
                .unwrap();
                at += size;
                index += 1;
            }
            send.finish().unwrap();
            connection.closed().await;
            server.close().await;
        });
        let client = endpoint().await.unwrap();
        let connection = tokio::time::timeout(
            Duration::from_secs(3),
            client.connect(EndpointAddr::new(peer).with_ip_addr(address), ALPN),
        )
        .await
        .unwrap()
        .unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();
        send.write_all(&[sample as u8]).await.unwrap();
        send.finish().unwrap();
        let start = std::time::Instant::now();
        let digest = tokio::time::timeout(Duration::from_secs(30), async {
            // Let more than 2048 packets collect before reading.
            tokio::time::sleep(Duration::from_millis(300)).await;
            let mut bytes = 0;
            let mut next_stall = 4 * 1024 * 1024;
            let mut digest = Sha256::new();
            let mut buffer = [0; 17333];
            while let Some(len) = recv.read(&mut buffer).await.unwrap() {
                for (index, value) in buffer[..len].iter().enumerate() {
                    assert_eq!(*value, block[(bytes + index) % block.len()]);
                }
                digest.update(&buffer[..len]);
                bytes += len;
                assert!(bytes <= TOTAL);
                if bytes >= next_stall {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    next_stall += 4 * 1024 * 1024;
                }
            }
            assert_eq!(bytes, TOTAL);
            digest.finalize()
        })
        .await
        .unwrap();
        assert_eq!(digest, expected);
        println!(
            "IROH_ROBUST_SAMPLE sample={sample} bytes={TOTAL} sha256={digest:x} seconds={:.3}",
            start.elapsed().as_secs_f64()
        );
        connection.close(0u32.into(), b"exact response drained");
        client.close().await;
        serve.await.unwrap();
        let dropped = counts[0].load(Ordering::Relaxed);
        let reordered = counts[1].load(Ordering::Relaxed);
        assert!(dropped > 0 && reordered > 0);
        println!("IROH_IMPAIRMENTS sample={sample} dropped={dropped} reordered={reordered}");
        forwarder.abort();
        assert!(forwarder.await.unwrap_err().is_cancelled());
    }
}
