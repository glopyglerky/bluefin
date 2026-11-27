# Bluefin Apple Rust core

Standalone nested workspace for the native Apple client of [Bluefin](https://github.com/glopyglerky/bluefin), based on b7d124a22d49bacf19f6c3c940333f508d647fb6. Repository-root attribution and GPL license apply. Swift/Xcode integration belongs to the parent task.

`include/bluefin.h` is one directory above this workspace. Link `libbluefin_apple_core.a`, expose that header through the Swift bridging header, and use the Apple SDK system libraries required by the final link. The simulator build reports native link inputs `-liconv -lSystem -lc -lm`; see the private native-static-libs log. Xcode app linkage has not been qualified by this worker.

## FFI contract

Expanded video capabilities are observations supplied in `videoCapabilities`,
schemaVersion1. HEVC and AV1 hardware observations plus `nativeProbeSupported`
allow bounded direct-play candidates beyond the legacy H264 SDR profile. The
returned `nativeProbeRequired` flag is mandatory: the native host must verify
that exact authorized asset with AVFoundation before installing the player.
On failure, drain the candidate and request one conservative fallback with the
same viewer, source, tracks and timeline. Do not infer a Dolby/HDR output mode
or lossless Atmos from decodability. Enhancement-layer Dolby Vision is excluded;
physical format/output qualification remains a separate release gate.

Local `stopPlaybackProxy` dispatch bypasses ordinary network-call saturation,
so closing a player cannot lose its local resource cleanup to `Client busy`.
Swift keeps an awaitable teardown owner and retries admission for remote
release for up to 60 seconds. Cleanup endpoints have a three-second request
timeout. Final progress and server release settle before remote logout.
Pending plans retain cleanup ownership until controller installation; cancelled
or rejected plans close their local proxy and release known server resources.
Rust also closes known opened live sources when preparation fails before
returning a plan. Source-only or host checks do not guarantee cleanup after
process termination or an unreachable server; such failures are surfaced.

`char *bluefin_call(const char *json)` blocks. Call it from a background thread, outside a Tokio runtime. Supply valid readable NUL-terminated UTF-8 of at most 65,536 bytes, excluding the terminator. The returned allocated UTF-8 JSON always has either `{"ok":true,"value":...}` or `{"ok":false,"error":"safe message"}`. Copy it, then release it exactly once with `bluefin_free`. NULL input is an error; freeing NULL is safe. Invalid/dangling pointers, double frees and foreign allocations violate the C contract and cannot be caught as Rust panics.

An Android `releasePlayback` error may also include a bounded `cleanupResources`
snapshot for the exact captured account. Independent remote acknowledgments are
retained even when local retirement or another remote step fails. This remains
`ok: false`; the snapshot is not a local-retirement receipt. Android protects
this progress before surfacing the error and retains the unfinished resource.
Other platforms keep their existing release behavior. Android release requires
its registered captured resource; sign-out restores protected pending receipts
after process recreation instead of operating on unregistered identifiers.

Panics during dispatch are caught. Four FFI calls may run concurrently; further calls fail with `Client busy`. Each HTTP request has a 5-second connection timeout, 15-second total timeout and 8 MiB body limit. Home performs three requests sequentially, so its bound is approximately 45 seconds. The final JSON envelope is limited to 8 MiB. Allocation exhaustion and host process termination are outside panic recovery. HTTP errors expose status only; transport and JSON errors never echo response bodies, credentials or URLs. The core installs no logging or global panic hook.

Common JSON fields are `operation`, `server`, `token`, `userId`, `deviceId`. Server must be an absolute HTTPS URL, optionally with a base path. Credentials, query and fragment are forbidden. Local HTTP requires `allowLocalHttp:true` and localhost, a `.local` hostname, or a private/loopback/link-local IP literal. The HTTP opt-in is intended for trusted local networks. Tokens are carried in the MediaBrowser Authorization header. No redirects, ambient proxy configuration, cookie storage or persistent credential storage are enabled. Swift owns Keychain persistence. Device ID defaults to `bluefin-apple`; supply a stable per-install ID.

| operation | Additional fields | value |
| --- | --- | --- |
| `info` | none; no token needed | raw System/Info/Public |
| `login` | `username`, `password`; no token needed | raw authentication result |
| `home` | `userId`, token | `{resume:[],latest:[],nextUp:[]}`; any failed component fails the call |
| `libraries` | `userId`, token | raw user Views |
| `items` | `userId`, token; optional `parentId`, `searchTerm`, `includeItemTypes` comma-separated string, `startIndex`, `limit` | `{Items:[],TotalRecordCount:n}` |
| `item` | `userId`, `itemId`, token | raw user item |
| `playback` | `userId`, `itemId`, token; optional `mediaType:"Audio"`, `startTicks`, `audioStreamIndex`, `subtitleStreamIndex` | playback object below |
| `stopPlaybackProxy` | `proxyId`; no server/token needed | `{stopped:true}`, idempotent |
| `started`, `progress`, `stopped` | token, `itemId`, `mediaSourceId`, `playSessionId`, `playMethod`; optional `positionTicks`, `isPaused`, `isMuted`, audio/subtitle indices | raw session response, normally null |
| `logout` | token | raw response; Swift clears Keychain after successful logout or explicit local sign-out |
| `quickConnectInitiate` | no token needed | raw initiation result |
| `quickConnectStatus` | `secret`; no token needed | raw connection status |
| `quickConnectAuthenticate` | `secret`; no token needed | raw authentication result |
| `irohProbe` | `peerId`, `peerAddress` numeric IP:port | feature-gated real handshake result, never media acceptance |
| `karaokeManifest` | `itemId`; optional `mediaSourceId` | validated authorized package metadata |
| `prepareKaraoke` | `itemId`, nonnil UUID `requestId`; optional `mediaSourceId` | validated preparation status |
| `karaokePreparationStatus` | same job fields | validated preparation status |
| `cancelKaraokePreparation` | `itemId`, `requestId` | `{cancelRequested:true}` after a successful DELETE; not a worker-join receipt |
| `karaokeStem` | `itemId`, `revision`, `role`, `sha256`, `bytes`, app-owned `destination`; optional `mediaSourceId` | verified temporary-file download |

Sing inference and model assets belong to the server. The client first checks
for an authorized package, then requests preparation when it is absent. A job is
bound to its item, client-generated request UUID and selected source. Only the
defined queued/preparing/ready/unavailable/failed/cancelling/cancelled states are
accepted. The server must authorize each request and retain cancellation before
a delayed POST; the request UUID is an idempotency key, not an access credential.
Cancellation uses the reserved cleanup path and a three-second request timeout.

Packages distinguish supplied stems from model-separated stems. Processed
packages require a public processing reference and a media source identity;
they do not establish original-audio fidelity. Selected source identity is sent
on manifest and stem requests. Stem transfers check the exact byte count and
SHA-256, reject redirects, and remove partial output on failure. The destination
is a native temporary path and is never transmitted to the server. Word timing
must be provided by the authorized package; the client invents none.

Additional operations are `favorite`, with `userId`, `itemId`, and a Boolean
`favorite`; `releasePlayback`, with `playSessionId`, `deviceId`, `playMethod`,
and optional `liveStreamId`; and `homeAssistantStates`/`homeAssistantControl`.
Favorite changes are scoped to the authenticated user's item route. Release
stops only the specified device/session transcode and closes an explicitly
identified live stream. The controller releases resources even if watch-state
reporting fails. Home Assistant uses its own selected endpoint and Bearer
token. State projection excludes raw attributes and private media URLs;
controls permit only play/pause, next, and stop on one `media_player` entity.

Items defaults to limit 50, maximum 200, start index at most 1,000,000. Ticks are nonnegative signed-64-bit compatible integers; stream indices range from -1 through 128. Use -1 to disable subtitles. Session reports use the original media timeline, including any validated HLS offset. Do not add startTicks twice after seeking. `started` maps to Sessions/Playing, `progress` to Sessions/Playing/Progress and `stopped` to Sessions/Playing/Stopped. Session failures propagate; the UI must handle or retry them explicitly. The core does not silently retry mutations. Quick Connect remains subject to server support and user approval through the server's normal flow.

Playback returns `playbackInfo`, raw selected `source`, local `url`, `remoteUrl` metadata, empty `headers`, `proxyId`, `playSessionId`, `mediaSourceId`, `startTicks`, `startOffsetTicks`, `playMethod`, and audio/subtitle indices. Give AVPlayer only `url`. No private AVURLAsset header property is needed. Never log raw upstream metadata, which may contain secrets. Call `stopPlaybackProxy` on player exit, replacement, cancellation and failure. Reporting `stopped` does not stop the proxy. Stop joins the proxy thread and cancels all its downstream and upstream sockets before returning. Stop must run on a background thread too. The capability URL and proxyId are sensitive local session capabilities.

For a server HLS URL carrying `StartTimeTicks`, `startOffsetTicks` is that returned offset, otherwise zero. Swift seeks to `max(0, desiredOriginalTicks - startOffsetTicks)` and reports `playerTicks + startOffsetTicks`. Do not also seek to the original resume ticks in an HLS stream that already starts there. Direct files have offset zero and require an ordinary seek. No timeline offset is inferred merely from the request. End-to-end server timeline behavior still needs native acceptance.

The profile requests H264 SDR up to 1080p, 8-bit, level 4.1, MP4 direct playback with AAC/AC3/EAC3, or HLS MPEG-TS with H264/AAC stereo transcoding. Missing direct-play metadata fails closed. Selected subtitles require server transcoding. For standalone audio pass `mediaType:"Audio"`. Audio-only sources prefer validated native MP3, AAC, FLAC, ALAC, or PCM before an offered lossy transcode. Native container/codec and output-route acceptance remain necessary. A lossless source does not prove a lossless Bluetooth, AirPlay, CarPlay, or DAC output. Sources without adequate direct metadata or an explicit transcode URL are rejected. The universal endpoint was inspected, but it creates another PlaySessionId internally; this implementation preserves the PlaybackInfo session and uses its validated source instead. This is a conservative plan, not a universal codec or server acceptance claim.

The bridge binds IPv4 loopback at a random port, with a 256-bit OS-random capability and opaque registered route numbers. It permits two live proxies and four active requests across them. Only GET/HEAD HTTP/1.1 requests, the exact loopback Host and at most 16 KiB headers are accepted. Requests cannot supply an upstream URL. Routes stay on the selected server origin and within the selected `Videos/{id}/` or `Audio/{id}/` directory, including any configured server base path. Relative playlist parents are normalized before this check. Encoded media paths, userinfo, fragments, foreign origins are rejected. Initial and nested playback credential query keys are scrubbed; ordinary segment parameters are preserved upstream. The user token is sent only in upstream Authorization headers. Redirects, cookies and ambient proxies are disabled.

Single byte ranges and upstream 200/206/416 are supported. Content-Length, Content-Range and Accept-Ranges pass through for media. Bodies stream with writes of at most 32 KiB and backpressure. Playlists are capped at 1 MiB, 16 KiB per line, and 4096 registered routes per session. Nested playlist lines and quoted URI attributes for keys, maps, media variants, session keys and iframe variants are rewritten to local routes. Duplicate/malformed attributes, non-identity key formats, encryption other than NONE/AES-128, variable substitution, content steering and unsupported URI extensions fail closed. Partial playlist responses are rejected; rewritten playlist lengths are recomputed. HEAD of a playlist omits the upstream length because rewriting changes it. Playlist responses are buffered only within their bound. Failed transfers after response headers close the connection.

Header reads time out after five seconds, upstream connects after five seconds, upstream reads and downstream writes after thirty seconds, and a request after six hours. The proxy remains owned until explicit stop so a failed stream can be retried. Native AVPlayer and the installed server journey remain parent qualification work, including loopback ATS configuration on iOS/tvOS 15. No physical or iOS15 runtime acceptance is claimed.

## Iroh scope

The core consumes the current `../iroh-modern` rlib inside its single static
archive. Both mobile compatibility patches are applied at this consuming
workspace root. Link only `libbluefin_apple_core.a` in the app.

Playback accepts optional `iroh:{peerId,peerAddress}` with an explicitly pinned
endpoint and numeric socket address. API metadata stays on canonical HTTPS.
The existing capability proxy validates and rewrites every HLS media route;
only upstream fetches map into the private authenticated tunnel. The usual
viewer Authorization is forwarded to the selected origin. No second token,
automatic peer replacement, discovery, relay or silent HTTPS fallback is used.
The session owns the tunnel until its proxy jobs drain on stop. See the modern
crate README for the bounded windows, request limits and foreground constraints.

The matching adapter supports explicitly admitted item prefixes and still
requires the origin to authorize every viewer request. Its CLI remains
foreground and ephemeral. Durable endpoint identity, trusted pin provisioning,
installed server integration, relays, suspend/recovery and mobile interface
migration need separate qualification. Host TLS/Iroh/HLS fixtures do not prove
native playback or remote-server acceptance.

The optional `iroh-probe` feature uses the same modern Iroh version for a real
pinned handshake with `bluefin/apple-probe/1`. Its `mediaRouteQualified:false`
result is only a probe. Workspace-gateway MCP framing is not a media protocol.
The retired compatibility experiment is not part of the active core graph.

### Streaming resource bounds

The proxy keeps at most 4,096 upstream routes per session. Sliding HLS windows
replace old segment mappings; expired local IDs are never recycled. The item
entry point remains available, and up to 256 recent rendition, key and
initialization references take precedence over old segments. A single manifest
that exceeds the active route budget is refused rather than publishing expired
URLs. This bounds memory without a lifetime segment limit for normal live
windows. Active responses use an idle read timeout, not a 30-second total media
deadline. Transport fixtures do not establish native decoder or provider access.

## Building and verification

Use a separate output directory, never this workspace as target storage:

```sh
IPHONEOS_DEPLOYMENT_TARGET=15.0 TVOS_DEPLOYMENT_TARGET=15.0 CARGO_TARGET_DIR=YOUR_BOUNDED_BUILD_CACHE cargo +stable build --locked --release --target aarch64-apple-ios-sim
```

Build `aarch64-apple-ios`, `aarch64-apple-tvos` and `aarch64-apple-tvos-sim` similarly after installing their stable rustup standard libraries and Xcode SDKs. Current default Mac toolchain is rustc 1.95.0. Keep Cargo.lock with source. A task-local CARGO_HOME avoids a pre-existing broken shared registry entry and is disposable after delivery.

Portable verification uses a canonical source checkout and task-owned
`CARGO_HOME`/`CARGO_TARGET_DIR` on the existing build filesystem. Run `cargo
test --locked` and `cargo clippy --locked --all-targets -- -D warnings`.
On supported hosts, also test `--features iroh-probe`. Mocks cover token-header
placement, escaping/base paths, credential-bearing and foreign URLs,
redirects, session/server errors, home failures, response/input limits,
unsupported sources, HLS/range streaming, cancellation, native lossless
selection, favorites, and Home Assistant scope. No household accounts are
used by these tests.

Lustr is not used: this core needs async dependencies, unsafe FFI and procedural macros outside its supported subset. A separate policy compiler experiment would not validate this core.

Retain libraries outside target directories, with one useful rollback and small
private logs/hashes. Exact deployment identifiers and operator evidence stay
outside product source. Deliverables do not establish physical-device or
installed-server acceptance.

## Music, guide and offline operations

`musicItems` returns paginated `Items` and `TotalRecordCount`. `kind` is `songs`,
`albums`, `artists`, `albumArtists`, `playlists`, or `playlistItems`. The latter
requires `playlistId` and preserves server order and `PlaylistItemId`. Optional
filters include `artistId`, `albumArtistId`, `albumId`, `parentId`, `isFavorite`,
`searchTerm`, `genres`, and `years`. Relationships use IDs, never title matching.
`startIndex` defaults to 0 and `limit` to 50 with a maximum of 200. Sort columns
are explicitly admitted; the default is `SortName` ascending. Favorites use the
existing `favorite` operation. Item details include chapters and media tracks.

`createPlaylist` takes `name` and optional `itemIds`, creates a private audio
playlist for the authenticated caller, and returns `Id`. `addPlaylistItems`
takes `playlistId`, `itemIds`, and optional `position`. `removePlaylistItems`
takes `entryIds` returned as `PlaylistItemId`, rather than inferring them from
track IDs. `renamePlaylist` changes only `name`; `deletePlaylist` checks the
current user's item type before calling the server delete endpoint. Permissions
and HTTP errors propagate. Mutations are never silently retried.

`lyrics` returns the server's `Metadata` and `Lyrics`, including optional tick
positions, word cues and timing offset. `liveChannels` returns the authorized
channel page and server `CurrentProgram`. `livePrograms` takes `channelIds` and
optional date bounds or `isAiring`, and returns programmes ordered by start date.
Use `minEndDate` for current and upcoming guide rows. The client applies no
sports policy or external metadata lookup.

`downloadStart`, `downloads`, `downloadCancel`, and `downloadRemove` take an
existing absolute app-owned `cacheRoot`, normalized server URL and `userId`.
Start also takes `itemId` and a token, verifies `/Users/Me`, checks the account's
download policy and admits a native original audio source via non-playing
PlaybackInfo. `/Items/{id}/Download` enforces actual server download permissions.
Redirects and foreign origins are refused. HLS and transcoded sources are never
cached as native audio. Content length, source length when supplied, and native
container signature are checked. A SHA256 digest verifies retained bytes.

One worker runs per process. Default per-file reservation is 256 MiB, configurable
through `maxBytes` to at most 1 GiB. Default account quota is 2 GiB, configurable
through `quotaBytes` to at most 8 GiB, with at most 100 records. A start reserves
the full per-file maximum against the quota; it can refuse a small file if that
reservation does not fit. Transfer buffers are 32 KiB and item metadata is capped
at 8 KiB. The account catalog is capped at 1 MiB. One staging file and a small
pending receipt permit bounded recovery after a killed process. Files are synced
and atomically renamed before becoming playable. Directory-FD operations reject
symlinks and non-regular or multiply linked files; account mutations take a native
exclusive lock. Caller must protect the app-owned root from concurrent renaming.

Records contain `itemId`, `state`, `bytes`, `totalBytes`, optional safe `item`,
optional generic `error`, and `localUrl` only for verified `completed` records.
States are `downloading`, `completed`, `failed`, and `cancelled`. Completed records
survive restart, while failed and cancelled progress is retained for the current
worker only. `downloads` returns `Items`, default limits, `maxWorkers`, and
`maxItems`. Cancellation and removal bypass network admission. Cancellation joins
the worker; an in-flight request may take its timeout, up to 30 seconds, to exit.
Logout joins the matching account worker before its server request. Account
namespaces contain no token. Native hosts must stop their player and discard file
URLs on logout; a file URL cannot revoke an already-open file descriptor.

Run the offline tests with `BLUEFIN_TEST_CACHE` pointing at bounded build storage.
No authenticated household account or live library mutation is needed by tests.

Apple delivery includes a real native C linkage check against each target
library. The installed Xcode linker accepts the default ThinLTO libraries. Its
`nm` reader reports unsupported LLVM attributes while inspecting newer Rust
bitcode, so that inspection alone does not establish a linker failure.

## Optional Bluefin and sports APIs

`bluefinCapabilities`, `bluefinMusicQueue`, `saveBluefinMusicQueue` and
`bluefinMusicQueueItems` consume the shared `bluefin-contract` v1 types.
Queue writes require `expectedRevision` and a typed `queue`. Conflicts remain
HTTP409 and are never automatically retried. Metadata fetches are user scoped,
batched and reordered to preserve duplicate tracks. Missing extensions return
normal errors; clients must not infer support from a server name.

`bluefinCapabilities` also projects the optional authenticated
`irohMediaTransport` advertisement through the shared transport DTO. The client
accepts version 1, ALPN `bluefin/http/1`, a 64-character hexadecimal endpoint ID and
up to 16 numeric direct addresses. Unsupported extensions are omitted without
disabling queue capabilities. This metadata grants no playback rights and makes
no reachability claim. Native transport settings can load it from the signed-in
HTTPS server; saving a pin remains an explicit action scoped to that account.

`sportsPreferences` and `sportsHome` use the selected origin's root
`/sports/api/v1/preferences` and `/sports/api/v1/home`. Saving preferences takes
the preferences object and the exact quoted numeric `etag`; PUT sends only
that object with If-Match. Missing/stale revisions remain HTTP428/412. The
adapter preserves unknown event identity and unverified availability. Native
server playback authorization remains authoritative.

Video playback accepts `mediaSourceId` and `quality` automatic/original/balanced/
dataSaver. Their bitrate ceilings are 80/200/12/3 Mbps; these are upper policy
bounds, not guaranteed throughput or codec support. The admitted codec profile
remains conservative SDR. Source and track choices are preserved when planning.
Explicit audio selection requests a transcode so static playback cannot ignore
it. Subtitle index -1 means off. A lower bitrate choice cannot silently select
a known over-limit static source. Source quality does not prove output quality.


Foreground Iroh proxy shutdown aborts and joins every proxy request before
checking exclusive session ownership and calling the transport's checked drain.
The proxy thread returns that result to `stopPlaybackProxy`; an unconfirmed drain
returns a safe error instead of `stopped: true`. Existing account cleanup callers
receive the same failure. Proxy admission remains locked through actual thread
exit. Drop remains a best-effort fallback for startup failures and unwind.
Call cleanup off the UI thread. Injected drain failures test receipt propagation;
real loopback HTTPS and pinned Iroh tests separately exercise transport behavior.

`tools/BuildCore.rs` remaps native C/ASM file, macro and debug source prefixes as
well as Rust paths. It enables cc's shell-escaped flag parsing only in child
commands and preserves inherited native flag arguments, including target-specific
flags. SDK selection and minimum OS 15 remain explicit. Inspect delivered static
archives with matching Rust LLVM tools; a bounded marker scan is not a universal
privacy or linked-app acceptance claim.

## Public music and regional access

Public search is available before pairing. The native Rust dispatcher owns
`publicMusicSearch`, `publicMusicDetail`, `publicCatalogOffers`, and
`publicCatalogCountries`. Apple and Android pass the same typed catalog identities.
Unpaired requests run through the public catalog clients. Paired Apple availability
uses the existing authenticated account transport, including mesh: structured Offers
POST preserves every asserted ID, and Providers GET supplies one exact title context
or a generic registry when the identity has multiple assertions. Invalid or retired
account metadata never falls back to an anonymous request. Swift request identity
includes the account lifetime, media identity, country and refresh attempt.

MusicBrainz supplies artist, release-group, release and recording identities,
descriptive metadata, related entities and actual service URL relationships. The
shared `MusicBrainzClient` uses one process-wide acquisition gate and bounded cache
for search, metadata and access. Search terms are escaped as literal text. Source
failures preserve their state and older cached observation time. Cover Art Archive
paths use real release IDs and may have no artwork. Artist photographs require a
matching MusicBrainz ID in Wikidata and credited Commons metadata.

Movie and TV access starts from unique, typed Wikidata TMDB cross-references.
Series cross-references are checked against the actual TVmaze or IMDb identity.
An external provider URL never supplies a Bluefin playback entitlement. Existing
authorized library playback stays ahead of external provider destinations.

The region picker starts with a valid device-region code, with no US fallback.
For unpaired video, the explicit all-regions view reads the upstream country dataset
and schedules at most three regional requests at a time. Paired all-regions access
makes one structured Offers request with a null country; a registry is not treated
as evidence of offers. Changing region, account or view cancels further scheduling. Progress, partial coverage and interrupted
sources remain visible. MusicBrainz links have unknown region; an explicitly
selected storefront can add exact-ID iTunes purchase/catalog listings. New provider
names and URLs are rendered from source evidence, including query-bearing links.

Qualification requires the existing Forge/native-owner pipeline. Refresh both
standalone native Cargo lockfiles under an admitted offline Forge job, retain those
files and freeze the source again before locked release qualification. No compile,
device or public-service success is implied by authored fixture tests.
