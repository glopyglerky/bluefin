# Progressive Bluefin web application

The ordinary web page now renders HTML before the optional Rust WebAssembly
module loads. Its Home, Library, Music, Search, Live TV, account, invitation and
player views use one DOM renderer across HTTPS and encrypted mesh connections.
It does not select a different visual interface when WebAssembly is unavailable.

## Native reference

The responsive styles use the application-owned native sources:

- Sources/BluefinStyle.swift: semantic system backgrounds, adaptive cyan
  accent, 12-point artwork corners, 1,200-point content width.
- Sources/AdaptiveNavigation.swift: welcome wordmark, 520-point account
  column, sign-in choices and connection labels.
- Sources/BluefinApp.swift: five main destinations, Home shelves, collection
  list, account entry and poster sizes.
- Sources/MusicDesign.swift and Sources/MusicPlayer.swift: music category
  buttons, 52-point song artwork, compact player, expanded artwork, transport,
  progress and queue controls.
- Sources/MeshPairing.swift and Sources/MeshConnectionsView.swift:
  invitation presentation and same-account device pairing wording.

The page uses system fonts and application-authored SVG paths. No Apple font or
SF Symbols asset is bundled. Width and input capabilities determine layout;
resizing retains navigation, queue, playback and the native media element.
Native Safari video/HLS presentation remains available. The page does not
invent immersive metadata or convert a flat video into spatial content.

This is source-level alignment, not a claim of verified pixel identity. Device
screenshots, touch/keyboard accessibility, iPhone dual-display transitions,
iPad/desktop layouts and Vision Pro presentation still require actual browser
qualification. Native Downloads, all Settings, integrations, lyrics and every
specialized native screen are not implemented by this web change.

## Capability boundary

index.html supplies a visible welcome screen, system-font styling and a
bounded startup message even if JavaScript assets fail. bootstrap.js lazily
tests a minimal Wasm module when the user chooses encrypted pairing. The probe,
module download and initialization each have a deadline. Full cryptography is
never rewritten into JavaScript, weakened, or substituted with an HTTP token.

The common view can connect to its own trusted HTTPS server, authenticate with
password, use its published passkey or Quick Connect flow, browse collections,
search, browse music and view permitted Live TV without Wasm, WebGL, IndexedDB,
a service worker or a system VPN. These are explicit HTTPS account sessions.
The server continues to make permission and content-access decisions in Rust.

A QR-launched device connection requires the existing Rust PQ adapter and an
independently configured approved relay. If these are unavailable, the
invitation is not redeemed over HTTPS. The UI keeps the same connection screen,
offers the installed application link for a valid-looking code, and explains
the unsupported capability. A QR code does not replace browser-trusted TLS.

## Native media without a worker

The HTTPS view discovers /Bluefin/Web/v1 and requires version 1 with
cookieMediaLease: true. Its ordinary authenticated request creates a selected
item lease using /Bluefin/Web/v1/leases. The response contains public,
same-origin scoped routes. The separate random media secret exists only in the
server's HttpOnly; Secure; SameSite=Strict cookie; it is never in JavaScript,
the DOM, query strings or browser storage.

The native media element receives the scoped route. Native HLS is preferred.
When required, the already-vendored same-origin HLS decoder can use MediaSource
without a worker. Unavailable decoding is reported without changing transport.
The server owns source selection, path validation, HLS resource rewriting,
authorization, cancellation and ongoing parent-session checks. The view renews
the lease while active and releases it on stop/sign-out. A page exit sends
best-effort cleanup, which cannot promise that the server received it.

For the verified previous server build, an authenticated 404, 405 or 501 from
the cookie capability endpoint permits a separate discovery of /Bluefin/TV/v1.
Only its complete version-1, 7,200-second absolute / 90-second idle lease
advertisement enables that adapter. A 401/403, malformed advertisement, timeout,
or failed cookie lease creation never selects it.

The older TV media route contains a secret capability for one selected item;
it contains no account bearer. It remains only in player memory and the native
media element, with no persistence, logging, copy/share UI or referrer. Its
route parser and lifetime are distinct from the public cookie route. DELETE
releases it; there is no POST renewal. A bounded HEAD while playing can keep
the idle lease alive but cannot extend its absolute limit. Resuming after a
long pause requests fresh authorization at the current playback position.
Servers with neither supported advertisement retain sign-in/library browsing
and report unavailable playback without guessing a stream URL.

Strict mesh media continues through the Rust per-request PQ protocol and the
existing client-bound service-worker bridge. It never calls the HTTPS cookie
gateway. If that browser cannot provide the worker/media capability, the paired
session remains paired and playback fails explicitly.

## Temporary devices

Mesh pairing uses an in-memory revisioned journal by default. Opening the page
does not open IndexedDB. The exact Begin identifiers and signed Finish request
are retained within that tab for idempotent retry. The key and invitation do not
enter localStorage, sessionStorage, cookies or URLs.

“Remember this device in protected browser storage” is a separate, unchecked
choice before pairing. It uses the existing AES-GCM encrypted IndexedDB journal
and nonextractable origin key. Restoring an older remembered device is explicit.
A blocked storage API prevents that choice; it never silently changes its
security or durability semantics.

Page retirement fences local operations, closes media and starts best-effort
remote revocation for temporary connected devices. JavaScript engines may stop
before a network acknowledgement arrives. The UI therefore tells users to use
Sign out for an acknowledged remote revoke, and does not describe tab closure as
a globally confirmed revoke. Temporary private keys disappear with the page;
server records still follow the server's actual expiry policy.

Remembered devices retain their protected retry material. Cross-tab journal
notifications apply only to that explicitly selected storage mode. Separate
tabs use separate outer iroh relay identities; the inner ML-DSA authenticated
device identity remains bound to its original saved key.

## Account and content boundaries

A “Connect another device to your account” QR is still the existing globally
one-use login invitation and requires the account's passkey. It is not a
different-person library grant, storage donation, or permission to reshare
content. Shared storage, content access and account membership need separate
authority. Neither physical deduplication nor possession of storage bytes may
grant playback, private search visibility or onward sharing.

No proprietary Omnidia implementation was copied into this application. The
adapter uses the existing public Bluefin Rust crate and server interfaces.

## Qualification

All product builds and tests run through the admitted Forge owner. The new
tests/progressive-web.test.cjs runs the actual renderer with Wasm, storage and
workers absent, verifies ordinary HTTPS browsing, rejects cross-origin/lease
escapes, checks no QR-to-HTTPS downgrade and checks temporary journal CAS.
These are controlled boundary tests. They do not certify Safari Lockdown,
physical playback, passkey enrollment, a live approved relay, or visual parity.

The web bundle now builds the optional Rust protocol adapter without the
eframe UI feature for the ordinary page. The old native/canvas implementation
remains source-compatible and is a separate regression compile target.

## Release consistency

The build-only package helper takes the admitted bindgen directory, a new output
path and the reviewed source root. It hashes the complete Rust engine (Wasm,
JavaScript glue and platform snippets) into one engine directory. Styles, shell
scripts and the vendored HLS decoder use content-addressed file names. Every
service-worker consumer uses the same worker hash in its registration URL.
The document base and worker scope remain at the original app path.

The package emits asset-manifest.json with each file's SHA-256 and byte length,
and fails when any asset exceeds 25 MiB. Deployment adds the exact corresponding
source archive and source revision from the admission receipt, updates SOURCE.txt,
and recalculates the final inventory. The account and welcome views link to that
source notice and the license. API routes on an assets-only host must return real
404 responses; they must never be rewritten to the application document.

## Temporary local music in the shared view

The welcome screen opens a local music library before account sign-in. It uses
the same song rows, mini player, expanded player, queue and adaptive navigation.
Connected accounts can open it from Music. Local search stays inside the File
adapter, and opaque local IDs never enter artwork, playback-report, invitation,
favorite or server-search requests. Remote playback stops and releases its lease
before the shared media element opens a local object URL. Removal, protected or
unsupported media errors, sign-out and page retirement release that source.

Only files selected with the browser picker are admitted. The temporary adapter
holds the original File objects and reads a bounded container prefix; it does
not upload, hash, deduplicate or persist the audio. This compatibility adapter
uses browser APIs because the interface must remain usable without WebAssembly.
Server and mesh authority, PQ cryptography and protocol handling remain in Rust.
Full browser codec support and physical Lockdown Mode behavior still require
actual device validation; passing the JavaScript boundary tests does not claim it.

The visible audio timeline, HTTP progress reports and seek controls use the same
transcode offset convention as the Rust playback service. Restoring a 30-minute
bookmark therefore does not report zero when the new media stream starts.

Playback domain changes await a command-specific Rust stop receipt. A previously
true cleanup flag cannot satisfy that barrier. If retirement is unconfirmed,
a second Play cannot bypass it. The browser adapter attaches an opaque owner
token to each media source; delayed cleanup, controls, metadata callbacks and
samples from a retired mesh owner cannot touch a local or HTTPS successor.
The Rust service remains owned through sign-out timeout until its lifecycle
actually finishes. Local search remains explicit inside Music even while an
account is connected, and a new explicit selection resets prior queue ordering.

The server-owned HTTPS lease profile transcodes audio to MP3 over HTTP and video
to H.264/AAC HLS. Audio therefore uses native audio/mpeg support without loading
an HLS decoder; only video uses the native-HLS or pinned MSE decoder path.

HLS playlists enumerate the full original timeline: their initial encoder seek
does not turn the playlist into a cropped stream. Video and mesh HLS therefore
use zero timeline offset and seek to the bookmark after metadata is available.
HTTP progress reporting waits for that initial seek instead of overwriting a
saved bookmark with zero. Progressive MP3 is a separate cropped stream whose
browser clock starts at zero, so only that profile adds its requested offset.
