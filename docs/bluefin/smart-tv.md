# Bluefin smart TV clients

This lane adds Roku SceneGraph, Samsung Tizen and LG webOS clients. Apple TV and Android/Google/Fire TV are separate clients. These sources do not establish support for Vizio SmartCast, VIDAA, NetCast, other closed TV stores, or every older model. No device enrollment, signing or store submission is part of this addition.

The web clients share protocol validation, queue transitions, guide time filtering, seek bounds and provider-link policy in `bluefin-tv-playback` compiled to WebAssembly. DOM focus, native playback, networking and lifecycle adapters use public platform APIs. Roku uses real SceneGraph/BrightScript and implements the corresponding bounded client behavior separately. Roku does not execute Rust or WASM.

## Client behavior

- Ordinary username/password sign-in uses `Authorization: MediaBrowser`, including a random app device identifier. Tokens stay in process memory and never enter player URLs. The web client stores only its nonsecret random device ID. HTTPS is required. A same-origin loopback exception exists only in the browser fixture adapter.
- Library, search, details, album browsing and music queues use existing viewer-scoped APIs. Web clients can explicitly load/save the account queue with revision checks. Roku's bounded queue is local to the app session. Neither client selects another viewer or enables administrative actions.
- Library playback uses a v1 server lease. Players receive only an opaque grant. Play/pause, seek, queue selection, now-playing progress and native track selection are implemented. Original source track selection creates a new negotiated lease. Backgrounding web clients stops playback and releases the lease; a later Play action obtains a fresh grant. Roku termination also relies on the server's abandoned-lease cleanup.
- Live TV displays the current authorized program guide. This v1 lease accepts local library items only. Live tuner playback, recording and sports write operations are not implemented here.
- User-activated provider buttons accept only exact HTTPS YouTube watch URLs with an eleven-character ID. Tizen uses public application control; webOS uses the public Application Manager browser launch. Roku displays the admitted public URL for use on another device. No undocumented cross-channel launch, provider credential forwarding, extraction, DRM handling or entitlement bypass is claimed.
- No HDR, Dolby Vision, Atmos or passthrough capability is advertised. The current server profile requests conservative H.264/AAC stereo HLS or MP3 through existing transcode policy. Roku queries current decoder support. webOS checks `canPlayType`; Tizen uses AVPlay and reports preparation failures. A denied transcode is shown as unavailable, never retried with elevated permissions.

## v1 contract

All paths are relative to the configured server base URL. Clients feature-detect `GET /Bluefin/TV/v1`; a `404` means playback is unavailable while ordinary authorized library browsing remains usable. The response is `{ "version": 1, "mediaLease": true, "absoluteSeconds": 7200, "idleSeconds": 90 }`. This discovery endpoint conveys protocol support, not permission to play.

`POST /Bluefin/TV/v1/leases` requires exactly one standard `Authorization: MediaBrowser ... Token="..."` header naming a current ordinary user session with a nonempty device ID. API keys, cookies, legacy bare-token headers and query credentials do not qualify. No query parameters are accepted. Maximum body size is 4096 bytes. Unknown fields at either level are rejected.

```json
{
  "version": 1,
  "itemId": "00000000000000000000000000000001",
  "playback": {
    "kind": "video",
    "maxBitrate": 8000000,
    "startTicks": 0,
    "audioStreamIndex": null,
    "subtitleStreamIndex": null
  }
}
```

`itemId` is a 32-character hex library ID. `kind` is `video` or `audio`. Bitrate is 128000 through 20000000. Start position is nonnegative and at most seven days in 100 ns ticks. Track indexes are optional unsigned 16-bit integers. The server validates actual item type and negotiated tracks using its existing planner. There are no account/user, URL, path, header, device-profile or upstream-credential selectors.

A successful `201` response contains:

```json
{
  "version": 1,
  "mediaUrl": "/Bluefin/TV/v1/media/<opaque-grant>/0",
  "releaseUrl": "/Bluefin/TV/v1/leases/<opaque-grant>",
  "playSessionId": "<server-generated-session>",
  "expiresInSeconds": 7200,
  "idleSeconds": 90
}
```

`GET` and `HEAD` on the returned media URL require no account header. The server retains the captured header privately and freshly authenticates it for each request. It re-reads the current user and Permissions projection, applies the existing account/schedule/network policy, then the existing current item/folder visibility checks. It repeats authentication and policy during an active response every three seconds. Resolved client address and proxy trust come from the actual request. Grants are also bound to the admitted resolved client IP; roaming requires a new grant. No loopback network request or local-authority substitution is used.

`DELETE` on the returned release URL idempotently cancels only that grant and its exact server-generated device/play-session encoding. It returns `204`, including after removal. A release grant cannot select another account or session. Idle expiry and router destruction also retire owned leases. The reaper needs a live Tokio runtime; process termination leaves final process/resource teardown to the server runtime.

The store is per router instance. Limits are 32 active leases, four per viewer, two per viewer/device, four concurrent admissions, 64 total media responses, eight per lease, 4096 fixed admitted resource routes and 1 MiB of route text per lease. Manifests and rewritten output are at most 1 MiB, references at most 4096 bytes, and lines at most 8192 bytes. A response uses at most one queued frame and one pending frame, each no larger than 1 MiB. These bounds exclude the existing server's own planner/transcoder internals, which retain their existing limits.

Headers/ranges are forwarded to the existing handlers. HLS HEAD requests authorize, load and rewrite the same manifest as GET before returning headers only. Playlist byte ranges return `416`; byte ranges for fixed media retain handler status/content-range behavior. No range support is invented for progressive transcodes whose existing handler returns `Accept-Ranges: none`.

Only exact fixed local Audio/Videos item routes with the admitted `playSessionId` can enter a resource table. Child references inherit the frozen negotiated query; only bounded numerical segment timing fields may be added. Canonical comma escaping in query values is normalized; encoded paths, encoded parameter names, nested escapes, traversal, off-origin/absolute references, duplicate parameters, redirects and credential parameters are rejected. Key/init/child requests go through the same authorization path. Unimplemented key or subtitle routes fail closed; external encrypted-provider manifests are not supported. Auxiliary trickplay, external subtitles and adaptive renegotiation are disabled. Account credentials never reach the output manifest.

Successful, failed and release responses set `Cache-Control: no-store, private`, `Referrer-Policy: no-referrer` and `X-Content-Type-Options: nosniff`. Adapters suppress backend error bodies and location headers. Built-in request spans and panic reports replace TV grant paths with route templates. Reverse proxies and any additional diagnostics must also avoid recording capability paths. Operations and stalled readers have a fifteen-second bound. Reader disconnect drops the upstream stream; expiry/release/revocation cancels active responses. The absolute two-hour bound requires obtaining a new grant for longer sessions; pausing longer than ninety seconds can also expire a grant.

## Required parent integration

Existing files are intentionally untouched. Integration needs:

1. Add `crates/bluefin-tv-playback` to workspace members and `bluefin-tv-playback = { path = "../bluefin-tv-playback" }` to `ferrofin-api` dependencies. Resolve the root lockfile once.
2. Declare `pub mod bluefin_tv_playback;` in the handlers module.
3. Implement `LocalSourcePins` at the library/media-source owner. `pin(item, source_id, expected_local_path)` must atomically compare and pin that local source generation, with every planner/stream/transcode resolver honoring the same pin until the returned guard drops. Source replacement must wait or be refused. A separate mutex or a best-effort post-check is insufficient. The adapter supplies no permissive implementation.
4. Call `bluefin_tv_playback::register(api, state.clone(), pins)` exactly once, before the normal outer IP/auth/CORS layers. It builds a private in-process router from existing media-info/video/audio/HLS handlers, supplies freshly authenticated user extensions, and preserves the resolved actual client address. No AppState field is added.
5. Optionally advertise v1 lease support in the existing Bluefin Capabilities DTO. Clients already use the narrow discovery route, so this is not a prerequisite. Review the existing CORS configuration for packaged TV web-app origins without weakening ordinary credential policy.

Source pinning is mandatory at admission and must retain the opened input through playback and encoder shutdown. Resolving a path twice does not establish that both requests use the same source. Validate this boundary together with an installed local-media playback journey.

## Packages and qualification

| Platform | Native implementation | Development artifact | Remaining acceptance |
| --- | --- | --- | --- |
| Roku | SceneGraph LabelList, KeyboardDialog, Video/Audio, Task networking, native player focus | BrighterScript-validated unsigned ZIP | Roku hardware/runtime, sideload, native player/track/remote/lifecycle checks, store certification |
| Samsung | TV remote DOM focus and public AVPlay/application control | Unsigned WGT archive assembled from config and assets | Tizen Studio/Web CLI validation, Samsung licensing/certificates, hardware/runtime, store certification |
| LG | Remote/pointer DOM focus, HTML media, public Luna Application Manager | Official webOS CLI unsigned IPK | Developer mode device/runtime, current engine/WASM and track APIs, store certification |
| Browser fixture | Same Rust/WASM and web app with original synthetic account/guide/audio | Static staging directory | Browser proof does not establish any vendor's installed acceptance |

The WGT is an unsigned archive, not a Samsung SDK-validated or installable/signed claim. The CLI package step does not establish LG device acceptance. No vendor proprietary SDK/partner license is accepted by the build steps here. The npm webOS CLI is Apache-2.0; BrighterScript is MIT. No vendor SDK implementation or premium assets are vendored. The application code and original icon follow the repository's GPL-3.0-only license.

Build Rust and WASM on the approved build host using Rust 1.99.1 and at most four jobs. An isolated qualification copy can add `[workspace]` to this crate's manifest, keeping the canonical root unchanged. Build client policy with:

```sh
cargo +1.99.1 build --manifest-path <isolated-crate>/Cargo.toml --no-default-features --target wasm32-unknown-unknown --release --lib -j4
```

For older WASM engines, the qualified build disables bulk-memory, reference-types and multivalue target features. Remap build-host source, registry and toolchain path prefixes before distributing WASM so panic/debug metadata cannot embed private host paths. Copy `bluefin_tv_playback.wasm`, the common `web/` files and one platform directory into a task-owned staging directory. For Tizen, include `$WEBAPIS/webapis/webapis.js` before `platform.js` in staged `index.html`; use its platform-specific player adapter and config. The webOS staging directory uses the common HTML player plus `appinfo.json`. Run official `ares-package <staging> -o <deliverables>` for IPK. With an already available authorized Samsung SDK, validate/build/package the Tizen project through its Web CLI. Do not sign or enroll automatically.

Roku's `bsconfig.json` sets deployment off. Run `bsc --project <roku>/bsconfig.json --outFile <deliverables>/bluefin-roku-dev.zip --stagingDir <cache>/roku`. Generated staging and packages belong outside canonical source. `tests/fixture-server.mjs <web-staging> <port>` serves original synthetic fixtures on loopback only. It has no upstream fetch or real-account route.

Focused tests cover strict admission DTOs, scope limits, grant isolation, real router HEAD/range handling, HLS child/key/init authorization and URL escapes, expiry, active revocation and reader cancellation. Adapter tests run the real ordinary-session, user-policy and item-visibility code with synthetic managers. They deliberately do not claim real database-session invalidation, transcoding/source-pin integration, or installed platform playback. Those remain parent acceptance checks.

## Public API references

- [Roku SceneGraph content metadata](https://developer.roku.com/dev/docs/content-metadata) and [Task HTTP transfers](https://developer.roku.com/dev/docs/ifurltransfer).
- [Roku current decoder capability APIs](https://developer.roku.com/dev/docs/ifdeviceinfo).
- [Samsung AVPlay](https://developer.samsung.com/smarttv/develop/api-references/samsung-product-api-references/avplay-api.html), [TV remote keys](https://developer.samsung.com/smarttv/develop/api-references/tizen-web-device-api-references/tvinputdevice-api.html) and [Web CLI](https://developer.samsung.com/smarttv/develop/getting-started/using-sdk/command-line-interface.html).
- [LG web engines](https://webostv.developer.lge.com/develop/specifications/web-api-and-web-engine), [Application Manager](https://webostv.developer.lge.com/develop/references/application-manager), [ACG declarations](https://webostv.developer.lge.com/develop/guides/acg-guide), [CLI installation](https://webostv.developer.lge.com/develop/tools/cli-installation) and [CLI packaging](https://webostv.developer.lge.com/develop/tools/cli-dev-guide).

The webOS manifest requests only the documented `application.launcher` ACG used by the public handoff. Its declaration is not proof of approval or entitlement. WASM, HTML media, fetch and encoding APIs are feature-detected. Older engines show an unsupported-runtime message instead of silently replacing shared policy. Hardware and firmware support must be measured on each claimed model family.
