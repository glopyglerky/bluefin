# Apple Iroh transport investigation

**Prototype only. Do not integrate or distribute this dependency graph.** The
older released dependency route compiles around the Apple `in6_ifreq` failure,
but retains RustSec findings. No accepted Apple library is delivered.

This standalone crate leaves the parent Rust and Swift projects unchanged.
It pins Iroh 0.95.1, netdev 0.38.2 and a local netwatch 0.12.0 patch that extends
existing iOS cfg branches to tvOS. It defines no new kernel ABI. The existing
upstream BSD parser is reused, without claiming its tvOS runtime correctness.
A second small portmapper 0.12.0 patch disables its three router-mapping defaults.
Both vendors retain upstream licenses and source. Cargo.lock fixes exact versions.
The older upstream lock seeded resolution because unconstrained prerelease
pkcs8/ed25519 dependencies fail to compile together. Rustls was subsequently
updated to 0.23.46 and rustls-webpki to 0.103.16. This does not clear the graph.

## Experimental contract

ALPN is exactly `bluefin/http/1`. Both ends must use this adapter and locked
Iroh generation. No compatibility with the parent's 0.98.2/1.4.1 endpoint or
another media protocol has been established. There is no discovery or relay.
A numeric peer address and pinned public peer id are mandatory. Direct
reachability is required. No HTTP fallback impersonates Iroh.

See bluefin_iroh.h for the background-thread start/stop/free ABI. Start takes
viewer authorization in memory and returns a random 256-bit capability URL on
IPv4 loopback. No credential persistence or credential query parameters exist.
Append the requested item media path to baseUrl. AVPlayer supplies no auth
header locally; the tunnel carries the configured viewer Authorization in HTTP
request bytes over the authenticated bidirectional QUIC stream, and the peer
forwards that same header to the origin. Stop is required on exit, replacement
and failure. It cancels tasks, closes the endpoint with a bounded grace period,
and joins the thread. Callers own valid FFI pointers and free each result once.
Never log configuration, raw requests, credentials or capability URLs.

The foreground `peer https://origin /Videos/item-id/` CLI prints only its public
peer id and addresses. It has no install or schedule behavior. The origin must
be a trusted, explicitly configured HTTPS origin without userinfo, path, query
or fragment. The item prefix must be `/Videos/<item>/` or `/Audio/<item>/`.
Origin redirects and environment proxies are disabled. There is no arbitrary
URL operation, alternate origin, URL token, or media server authentication
bypass. Synthetic tests add a fixture CA privately; production trusts normal
HTTPS roots and never disables certificate verification.

GET/HEAD only, one request per connection/stream, no body, canonical request
encoding, exact Host, no duplicate headers, at most 32 headers and 16KiB total.
Paths reject encoded characters, traversal, backslashes and credential query
keys. Only a single bytes range is allowed. Responses preserve 200/206/416,
401/403/404 and selected content/range headers. Media is streamed with 32KiB
writes and backpressure. This is a byte transport, not HLS playlist rewriting,
codec negotiation or a player session manager. Parent integration must keep
nested URLs on the authenticated transport and qualify actual playback.

Bounds are two local tunnels, four local requests globally, four peer requests
per adapter, one bidirectional QUIC stream, zero unidirectional streams,
64KiB stream receive window, 256KiB connection receive/send windows, five-second
headers/connect, 30-second idle reads/writes and six-hour total requests.
Malformed requests fail closed by dropping the connection. No raw errors go
through FFI. Local disconnect and explicit stop cancel upstream requests.

## Evidence and limitations

NAS tests use only a generated local HTTPS fixture and synthetic Authorization.
They exercise real Iroh GET, HEAD, ranges and 416, peer-pin mismatch, auth
rejection, path restrictions, actual C ABI start/free/stop, capability rejection,
header size and concurrency limits, disconnect and four-stream cancellation.
Two test functions contain these checks. No production service or media was
used. No Apple runtime, physical device, OS15 runtime, app link, CarPlay,
installed peer, native player or full media journey is accepted.

The parent receives exact source hashes, dependency patch/provenance, build and
security evidence in the private IROH-HANDOFF.md. Generated output is disposable.
