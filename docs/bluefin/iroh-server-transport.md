# Built-in Iroh media transport

The transport is disabled by default and can be enabled with trusted process
configuration. It uses a server-only, licensed Iroh 1.4.1 extension enforcing a
fixed network path. Noq rejects changed-address packets before processing their
application frames. A network change requires a new connection and fresh
admission; clients cannot retain the previous address's network exemptions.

This is source and synthetic Linux qualification. Installed services, native
clients, other platforms and public-network reachability need separate proof.

The production server compiles `bluefin-iroh-server` by default. It stays off
unless trusted process-start input explicitly enables it. TCP and HTTPS remain
the control plane for login, source selection, playback/session control, lease
admission, explicit stop and capability discovery. No localhost HTTP adapter,
service credential or remote-origin proxy participates in Iroh dispatch.

## Startup and identity

`BLUEFIN_IROH_CONFIG` names an operator-managed JSON file, at most 16 KiB, with
exactly these fields:

| Field | Meaning |
| --- | --- |
| `secret_key_file` | Absolute normalized filename inside an existing owned 0700 directory |
| `bind_address` | One concrete local UDP socket address chosen by the operator |

The configuration is read once per process, including across API-triggered
in-process restarts. Changing it requires a cold process restart. Its absence
does not open an Iroh UDP socket or identity file. An invalid opted-in setup
fails startup; it does not silently fall back to an ephemeral identity or a
wildcard socket. No product API writes this configuration.

Directory traversal opens each component without following symlinks. The
identity must be an effective-user-owned regular single-link 0600 file. Creation
is exclusive, writes 32 raw secret bytes and synchronizes file and directory.
An exclusive file lease lasts through joined endpoint shutdown. A partial or
unsafe existing file is refused without replacement. Reopening the same valid
file preserves the public EndpointId. The secret, private paths and bind values
are not included in product log messages added by this transport.

Only the configured socket is bound; the resulting bound socket is checked
against that input. Relays, address-lookup services and port mapping are
disabled. There is no discovery server, router configuration or global host
network change. The intended initial scope is a directly reachable fixed
single IP path on Linux. NAT traversal, relay reachability,
Internet accessibility and other host platforms are not acceptance claims.

## Protocol and authority

The selected graph uses Iroh **1.4.1**, its explicit **tls-ring** provider and
Noq **1.4.0**. The root selects the same licensed modified `noq-proto` path as
the native transport; there is no second protocol implementation or renamed
version. The server-only Iroh copy is `vendor/iroh-server-1.4.1`; its provenance,
licenses, API and update obligations are in `BLUEFIN-SERVER-PATCH.txt`.
Its `fixed_path()` builder policy disables ordinary migration, handshake
migration, multipath and QNT coherently, including replacement server configs.
No independent client graph or reviewed Noq bytes are changed.

ALPN is `bluefin/http/1`. Each pinned connection carries one bidirectional
stream, one HTTP/1.1 GET or HEAD request with `Host: bluefin`, followed by client
request FIN. There are no request bodies or pipelined requests. The client must
drain the response through FIN before closing the connection. QUIC ACKs do not
prove application consumption, so the server retains the connection, body,
source and admission ownership through application close or termination.

The Iroh EndpointId authenticates the transport peer. It supplies no account,
device, LAN, library or media-operation authority. Every request needs ordinary
`Authorization: MediaBrowser ... Token="...", DeviceId="..."`. The production
fresh-session seam checks the actual persisted token, device binding, current
account and current policy. API keys, legacy bare-token headers, query tokens,
caller-selected users and forwarding headers cannot authorize Iroh media.

The observed incoming socket must match the sole open selected direct path
reported by the current Iroh connection APIs. Local IP observation must match
the actual bound socket. This context is inserted as `ConnectInfo` before the
production IP/auth layers. Relay, missing, ambiguous and mismatched contexts
are refused. The path-event subscription precedes the snapshot; notifications
describing that same snapshot are checked against it. Changed, closed, lagged
or unobservable path events close the connection. Same-path rebinding is
rejected inside Noq before frames can update the connection or move queued
output; cached getters and event observations supply no enforcement boundary.
For a connection without multipath, Iroh selects that connection's own path
zero rather than another retained connection's peer-wide preferred tuple.
Iroh requests never consult peer-supplied proxy headers.

Packet rejection cannot promise a wire-visible close when the old return
route is unavailable. The encrypted UDP qualifier retains the original
five-second observation and 8/16/16 MiB windows, drains more than 8 MiB before
each switch, discards old-socket replies, and checks zero replacement-socket
replies and no progress beyond pre-switch buffered credit in both directions.
It separately proves a new same-identity handshake observes the changed tuple
and joined stop releases body/source/permit/socket owners. Deadline expiry is
never reported as closure. Quiet lost-path owners retain the existing bounded
30-second idle/write termination; explicit shutdown cancels and joins them.

Dispatch uses the fully composed production API router and existing handlers.
For ordinary item media, the existing production source-pin factory rebinds
only source-consuming services to an actual opened finite local input. The
source-bound view shares the ordinary account, configuration, filesystem,
encoder queues and coordination owners. Existing TV media retains its already
admitted source-bound lease and original router. Router-private path parameters
are not forwarded into the source-bound dispatch.

Current session, device, network and pinned-source policy are rechecked during
streaming, including during a blocked output write. Ordinary item media additionally retains
the opened file and compares its device/inode/length/mtime/ctime stamp. A change
terminates output. This is cooperative mutation detection, not immutable bytes,
a filesystem snapshot, or a hard deadline for an uninterruptible kernel call.
The existing `current_pinned_policy` and karaoke's stricter
`current_local_audio` contracts are separate and unchanged.

Ordinary HTTP playback admission now enforces the existing
`EnableMediaPlayback` boolean, including direct GET, ranges, HEAD and HLS
admissions through `require_streamable_item`. Static response bodies read this
boolean from the owning database before every next body frame, retaining
their actual source until cancellation or completion. Bytes already handed
to a transport cannot be recalled. This does not alter credential parsing,
legacy authentication, API-key handling, catalog/guide policy or login.

## Admitted media surface

Only canonical reviewed routes are dispatched:

- `/Audio/{itemId}/stream` and reviewed `stream.{container}` variants;
- `/Videos/{itemId}/stream` and reviewed `stream.{container}` variants;
- ordinary audio/video `master.m3u8`, `main.m3u8`, video `live.m3u8`, and
  reviewed `hls`/`hls1` segment routes for finite pinned inputs;
- `/Bluefin/TV/v1/media/{grant}/{resource}` for an existing lease.

Iroh lease requests must present that lease's current viewer/token/device.
The original HTTPS bearer-capability behavior has its separate existing owner.

Item IDs must be actual non-nil UUIDs. The ordinary media handlers still own
source selection, range/HEAD/length behavior, encoding and session policy.
Non-static ordinary media requires an ordinary playback session identifier;
the device comes from the current authenticated session, and a supplied device
query must match it. Queries accept only the reviewed playback fields. URLs,
filesystem paths, live-stream selectors, arbitrary catalog/admin routes,
redirects and credential queries are refused. Remote sources, shortcuts, discs,
multipart/external auxiliary sources and inputs the production pin factory
does not support fail closed. Subtitle, trickplay and other auxiliary routes
are outside this media surface. A manifest reference to an unsupported route
does not authorize it.

Only Host, ordinary Authorization, Range, If-Range, If-Modified-Since,
If-None-Match and Accept reach the router. Standard response media/range/cache
headers are serialized without redirects or whole-media buffering. Bodies use
bounded producer frames and writes of at most 32 KiB. The client must still
apply HTTP status and length semantics.

## Bounds and lifetime

Headers are capped at 16 KiB and 32 fields, with five seconds for handshake,
stream opening and complete headers plus request FIN. Router response startup
and stream idle intervals are 30 seconds; connection lifetime is at most six
hours. These limits are fixed. Four connection permits cover handshake,
request, body and drain. The fifth connection is refused without a waiting
queue. Bidirectional stream admission is one, unidirectional streams and
application datagrams are disabled. Stream/connection receive windows are
8/16 MiB and send window is 16 MiB. These are protocol flow bounds, not a total
process or kernel memory quota.

The server owns the accept loop, every connection task, cancellation signal,
endpoint and persistent identity lease. Normal shutdown cancels and joins every
connection, awaits endpoint closure, clears the running projection and only
then lets the host tear down shared managers/database state. `StopReceipt`
counts joined connection tasks and records completed endpoint closure; it is
not an application delivery receipt. Existing encoding/TV lifecycle and
explicit stop APIs retain their original owners and stop contracts.

## Capability projection

Authenticated `/Bluefin/Capabilities` may add optional `irohMediaTransport`:
`version`, public `endpointId`, `directAddresses`, `alpn` and `directOnly`.
It is omitted while disabled/stopped or when the current viewer fails the
fresh playback/network disclosure checks. Existing fields and older DTOs are
unchanged. Hints identify an actually running bound endpoint; they contain no
secret seed, route-prefix grant, account entitlement or reachability promise.
Every subsequent media request authenticates and authorizes independently.

Qualification receipts describe exact source/runtime evidence separately from
installed service, public network, native application and release acceptance.
No deployment or whole-release acceptance follows from a source build.
