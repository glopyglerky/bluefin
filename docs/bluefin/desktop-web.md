# Desktop and browser client

`apps/bluefin-universal` is an independent Cargo workspace for a Rust eframe desktop application and a WASM browser application. They share account validation, PlaybackInfo planning, playback lifecycle and queue policy. This implementation still needs real-server and platform qualification before release.

## Build and run

Use Rust 1.99.1 and an owner-assigned target directory on the build filesystem. From `apps/bluefin-universal`:

```sh
cargo +1.99.1 fmt --check
cargo +1.99.1 test --locked
cargo +1.99.1 clippy --locked --all-targets -- -D warnings
cargo +1.99.1 build --locked --bin bluefin-universal
"$CARGO_TARGET_DIR/debug/bluefin-universal"
# Default: embedded OpenGL video. Explicit software-video/WGPU compatibility path:
"$CARGO_TARGET_DIR/debug/bluefin-universal" --wgpu
cargo +1.99.1 build --locked --lib --no-default-features --features ui --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir dist "$CARGO_TARGET_DIR/wasm32-unknown-unknown/debug/bluefin_universal.wasm"
cp index.html media-worker.js dist/
cp -R vendor dist/
```

For a server-hosted Linux build, use wasm-bindgen CLI 0.2.126 and run `sh scripts/build-bluefin-web.sh <new-output-directory>` with an external `CARGO_TARGET_DIR`. The generated snippets include `web-player.js`. Install the directory as `bluefin` inside the server's configured web directory alongside its existing `index.html`. Open `/web/bluefin/` over HTTPS, or prepend the server's configured base path. Bluefin fills in the hosting server address, including that base path. Use **GPU view** to open the WGPU renderer and **Accessible view** to return to the browser-native controls.

When connecting to another server, that server's CORS policy must allow `Authorization`, JSON requests and Range, and expose Content-Range, Content-Length and Accept-Ranges.

Desktop playback loads an installed compatible **libmpv**, with the client and software-render APIs available in mpv 0.35 and newer. Use a maintained platform package and its matching codec dependencies. `BLUEFIN_MPV_LIBRARY` can select a library explicitly. Default library names are `libmpv.so.2`, `libmpv.2.dylib`, and `mpv-2.dll`, with compatibility fallbacks. Missing libraries produce an actionable playback error. No browser companion or external media process is launched. Packaging must include a compatible runtime or document that dependency. macOS packaging must retain a macOS 15 minimum; no private Apple API or entitlement is used here.

The [libmpv client API](https://github.com/mpv-player/mpv/blob/master/include/mpv/client.h) and [software render API](https://github.com/mpv-player/mpv/blob/master/include/mpv/render.h) define the narrow Rust adapter. libmpv and its FFmpeg dependencies contain native code and unsafe boundaries. This application makes no whole-stack Rust or memory-safety claim. mpv builds may be GPL or LGPL according to their build configuration and dependencies. Preserve the exact runtime's licenses and corresponding source obligations when distributing it. The Bluefin application is GPL-3.0-only.

## Library, account and discovery behavior

Sign-in uses the standard `Authorization: MediaBrowser` scheme, including client, truthful platform/device description, a random per-instance DeviceId, version, and a validated, quoted and escaped Token parameter. API requests, native media, optional Iroh configuration and worker media all use this scheme. Legacy headers are never sent. Credentials are memory-only. They do not enter URLs, DOM attributes, logs, cookies, local storage or distribution files.

Capabilities must match the authenticated viewer and protocol version. The UI browses movies, series/episodes, albums/tracks and the live guide. A program can tune its returned ChannelId; channel rows can play directly. Queue edits use the current `MusicQueue`, `QueueState` and `UpdateMusicQueue` contract, including expected revisions and visible conflict errors. Duplicate tracks retain their individual indices. Playback started from a queue advances on confirmed EOF, supports repeat and shuffle, visits each queued index once before a shuffled Repeat Off run ends, and stops sequencing after cancellation or account replacement. The playing queue is a snapshot taken when queue playback starts; later edits apply to the next explicit queue start. Cursor movement during playback is local and does not silently overwrite the account queue.

Discovery consumes only generic capability, search, form and action DTOs. Actions require the authorized-use acknowledgement supplied by that contract. No provider administration, provider SDK, upstream credentials, provider configuration or acquisition integration is included in this native/WASM distribution.

Browsing requests and completions have cancellation/epoch guards. Playback has an independent lifecycle, so switching tabs does not cancel progress reports or cause an old EOF to start another account's queue. Sign-out immediately stops local output, cancels browsing and playback, clears account state, drains prepared playback responses, releases scoped live/transcode resources, and requests `POST Sessions/Logout`. Another sign-in waits for that retirement attempt to finish. The UI retains failures and reports cleanup/revocation as acknowledged only after the corresponding responses. Network failure, application termination and canceled sign-in responses cannot establish server revocation. Closing a browser page attempts local worker cleanup; explicit sign-out is the acknowledged server-revocation path.

## Shared playback contract

`playback.rs` adapts the compatible media API locally, without adding a second server authority or modifying the shared contract. It sends `POST Items/{id}/PlaybackInfo` with the user, actual platform profile, position, selected audio/subtitle indices, playback intent and automatic live opening. It validates the returned play session, source, track list and item-pinned URLs. Unsupported preparation fails visibly and attempts to release prepared resources.

The planner chooses negotiated progressive audio/video or HLS. Track changes negotiate a new scoped plan at the current position. Explicit audio or subtitle selection requests server transcoding/burn-in, rather than claiming that HTML can decode every embedded subtitle format. The server must return a compatible selected stream; access or codec failure stays visible. Live streams use their negotiated source and live-stream ID. Live seeking is disabled because this client does not claim a seekable archive.

One Rust service owns started/progress/stopped reporting, prepared-resource cleanup, account retirement and queue sequencing for both backends. Progress reports run about every five seconds and after control changes. Reports include item/source/play-session IDs, position, pause state, selected streams, seek support and play method. Start is reported only after media time advances. Cleanup sends a stopped report, closes the selected live stream, and deletes only the plan's active encoding using its DeviceId and PlaySessionId. Prepared unselected live sources are released too. Failed acknowledgement does not become a success message or automatic queue progression.

## Embedded native media

libmpv decodes and outputs audio inside the application process. The default Glow renderer embeds libmpv's OpenGL render API in the player viewport using an RGBA8 framebuffer and GPU blit, without full-frame CPU copies. Physical viewport pixels include HiDPI scaling; allocations preserve aspect and are bounded to 4096 × 2160. `--wgpu`, an absent GL owner or failed GL initialization selects a source/display-aspect-aware software mailbox bounded to the same dimensions. Its CPU cost can be substantial at 4K. See [desktop rendering](desktop-rendering.md) for ownership, fallback and acceptance limits. Reporting keeps its separate, slower tick. Player controls provide pause/resume, seek, stop and negotiated audio/subtitle choices. Hardware decoding is requested with `auto-safe` on OpenGL, but the player reports the observed `hwdec-current` property. This configuration alone proves no hardware use. Source codec/dimensions, audio rate/channels and source transfer are shown separately from the observed render path. RGBA8 SDR output is not native HDR or Dolby acceptance.

A random item-scoped loopback bridge keeps account credentials out of the decoder. It checks Host, Origin, method and the exact registered capability route. It forwards Range and streams progressive bytes with backpressure, at most four concurrent requests and no short total media deadline. Headers and idle reads have ten-second bounds. Stop cancels upstream work and retires the listener independently of the UI thread. API operations remain bounded separately. HLS playlists have a 1 MiB ceiling; all URI lines and quoted URI attributes are rewritten through origin/item checks. A 2,048-entry resource map retires old entries while preserving the entry playlist, allowing live playback without unbounded map growth. Redirects, upstream cookies and credential query overrides are refused.

The native decoder permits HLS references only for a negotiated HLS plan, after rewriting, and forces the HLS demuxer in that case. Other media keeps reference loading disabled. Decoder scripts, user configuration and external URL helpers are disabled. Network protocols and codec buffering are limited. This is a narrow adapter to a native media library, not a sandbox or DRM implementation.

Native media can optionally use the existing pinned `bluefin-iroh-modern` transport, ALPN `bluefin/http/1`. Authentication and catalog remain canonical HTTPS. A selected direct route never silently retries through HTTPS. Its existing idle and six-hour transport limits still apply; this lane does not alter the transport. The local URL adapter retains the returned opaque capability prefix. The transport's checked `stop_with_receipt` joins decoder retirement and tunnel drain. Direct cleanup and automatic EOF queue progression are admitted only after successful retirement and an acknowledged receipt; failures remain visible. Browser media uses HTTPS, with a clearly labeled loopback exception for synthetic fixtures. Browser playback is never labeled Iroh.

## Browser media

HTML media supplies progressive decoding and native HLS when available. The vendored Apache-2.0 [hls.js 1.7.4](https://github.com/video-dev/hls.js/tree/v1.7.4) light build supplies the MediaSource fallback. Its license is in `vendor/hls.LICENSE`; its exact bundle hash and origin are recorded in `vendor/README.md`. No CDN request is needed at runtime. Fallback buffering targets 20 seconds forward, ten seconds back and a 32 MiB media buffer; browser/decoder/GPU allocations are additional platform-owned memory, not a claimed hard process quota.

The service worker receives the authorization header through an acknowledged MessageChannel. It binds that memory to the sending WindowClient, generates a fresh opaque media namespace, and pins every upstream resource to the negotiated server origin and item prefix. Another client ID, a retired namespace, a foreign origin/path, redirects, credential query overrides and ambient cookies are rejected. Range responses stream through a reader with backpressure; movies are never materialized as a full Blob or byte array. Only playlists are buffered, with the same 1 MiB ceiling. Media reads have an idle timeout, not a 30-second total deadline.

The worker limits active requests and resource mappings, and uses a renewed 60-second lease for abandoned clients. Replacement or clearing aborts readers, retires capabilities and clears authorization memory before acknowledging. The Rust lifecycle waits for that acknowledgement on stop/sign-out. Browser autoplay restrictions remain visible and the native HTML Play control remains available. Offline playback, DRM, casting and provider entitlement systems are outside this implementation.

## Original fixtures and checks

The fixture tools use invented metadata, generated moving test patterns, quiet synthesized tones, progressive MP4/WAV, VOD HLS and a sliding live HLS playlist. They never read personal media or contact an external media service.

```sh
cargo +1.99.1 build --locked --no-default-features --examples
# ffmpeg must already be available for generation.
"$CARGO_TARGET_DIR/debug/examples/fixtures" "$FIXTURE_DIR"
"$CARGO_TARGET_DIR/debug/examples/fixture_server" "$WEB_DIST" "$FIXTURE_DIR" 18461
node --test tests/media-worker.test.cjs
# Run opt-in decoder tests only against the loopback fixture server.
# Set BLUEFIN_FIXTURE_BASE and BLUEFIN_MPV_LIBRARY first.
BLUEFIN_TEST_NULL_AUDIO=1 cargo +1.99.1 test --locked --no-default-features --test decoded_media -- --ignored --test-threads=1
cargo +1.99.1 test --locked --no-default-features --test stream_bounds -- --ignored --test-threads=1
```

The fixture server creates ephemeral synthetic sessions and rejects legacy auth and query-token credentials. `Explore original fixtures` signs into that loopback fixture API. The native test base defaults to loopback port 18461 and can be changed with `BLUEFIN_FIXTURE_BASE`. Production sign-in continues to require HTTPS. The fixture evidence endpoint contains only synthetic reporting/account counters. It is a local test tool, not a production API or media server.

Tests distinguish protocol/state checks, actual media decoding, browser playback and installed acceptance. Native runtime checks decode video and audio, observe advancing time and nonempty rendered frames, seek, decode HLS/live, exercise EOF queue progression, retire the bridge/decoder and verify revoked synthetic credentials are rejected. Null audio output verifies decoding, not a physical speaker route. Separate timing checks cross the former 30-second deadline and exercise idle reads and explicit cancellation. Worker security tests cover account/client isolation and pinned HLS resources; they do not replace actual browser decode checks.

## Integration and qualification

No root workspace, shared FFI dispatch, Apple project or Android project change is required for the independent executable. Parent integration owns packaging, runtime libraries, root gates, real servers and device sessions.

| Boundary | Entry |
| --- | --- |
| Desktop executable | `src/main.rs::main` |
| WASM entry | wasm-bindgen start in `src/lib.rs` |
| Shared planning/lifecycle | `playback.rs`, `service.rs` |
| Native backend | `player.rs`, `mpv.rs`, `bridge.rs` |
| Browser binding | `web-player.js`, `media-worker.js` |
| Native direct route | Existing `bluefin_iroh_transport::{start,stop_with_receipt}; conditional admission after successful retirement` |

| Target | Qualified by this lane | Remaining gate |
| --- | --- | --- |
| Linux x86_64 | Rust build/checks and actual headless video/audio/HLS/live decoding, seek, queue, cancellation and synthetic session lifecycle | Display/audio-device execution, installed bundle, real authenticated server and pinned Iroh journey |
| macOS arm64 | Synthetic fixture CLI build with macOS 15 deployment target for browser replay | Desktop executable/runtime bundle, native display/audio and installation |
| macOS x86_64 | Source adapter only | Build, runtime dependencies and execution |
| Windows x86_64/arm64 | Source adapter only | Toolchain, matching libmpv DLL/dependencies, packaging and execution |
| Linux arm64 | Source adapter only | Build, runtime dependencies, display/audio and execution |
| Browser wasm32 | Current WASM build/lint checks; retained prior web package was not regenerated. Older T3 browser replay belongs to the prior handoff | Current glue packaging/replay, real-server HTTPS/CORS, actual alternate audio/subtitle server encoding, browser/platform matrix, fallback renderer and accessibility |

Exact candidate hashes, browser event evidence, initial failures and output cleanup receipts belong in the private worker handoff, outside public product source. No source check, synthetic fixture or single successful request establishes installed or remote-provider acceptance.
