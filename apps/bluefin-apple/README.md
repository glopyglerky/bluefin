# Bluefin for Apple devices

A native SwiftUI client with a Rust protocol and playback core. The source is
under development. There is no published app, TestFlight build, or complete
device qualification yet.

## Implementation

Rust validates server addresses, authenticates users, reads the library,
negotiates a conservative device profile, and reports playback sessions. The
Swift shell uses Keychain, SwiftUI, AVPlayerViewController, AVAudioSession, and
the system Now Playing and remote command APIs. Artwork uses a bounded memory
cache, same-origin authenticated requests, and native thumbnail decoding.

There is no JavaScript runtime, web view, bundled FFmpeg, or software video
decoder in this client. Unsupported media must be remuxed or transcoded by the
server. That decision trades broader direct-play format coverage for native
playback and a smaller client. Hardware-specific HDR, Dolby Vision, Atmos, and
AV1 support need real-device capability checks before being advertised.

The app has separate iOS and tvOS targets. The deployment target is iOS 15 and
tvOS 15. This describes the intended OS floor, not proof of compatibility with
every device. Older 32-bit iOS hardware and Apple TV generations without tvOS
cannot run these targets. Simulator success cannot qualify hardware decoding,
HDR output, AirPlay, PiP, or Siri Remote behavior on a physical Apple TV.

## Build with Xcode

Install Xcode and XcodeGen. The Rust workspace is standalone, so building this
client does not compile or link the Ferrofin server workspace.

Build the Rust library for the exact destination, then generate the Xcode
project with `xcodegen generate`. Open `Bluefin.xcodeproj`. Set
`BLUEFIN_RUST_LIB_DIR` to the directory containing the matching
`libbluefin_apple_core.a` using an untracked local configuration or the
`xcodebuild` command line. Choose the Bluefin scheme for iPhone/iPad or
BluefinTV for Apple TV. Personal signing/team identifiers stay outside source.

Supported Rust artifact targets are `aarch64-apple-ios`,
`aarch64-apple-ios-sim`, `aarch64-apple-tvos`, and
`aarch64-apple-tvos-sim`. Intel simulator libraries have not been delivered.
See [the Rust contract](rust/README.md) for build and FFI details.

For a simulator build, for example:

```sh
xcodegen generate
xcodebuild -project Bluefin.xcodeproj -scheme Bluefin \
  -destination 'platform=iOS Simulator,id=YOUR_DEVICE_ID' \
  -derivedDataPath YOUR_BOUNDED_BUILD_CACHE \
  CODE_SIGNING_ALLOWED=NO \
  BLUEFIN_RUST_LIB_DIR=YOUR_ARM64_IOS_SIM_LIBRARY_DIRECTORY build
```

Run the Bluefin scheme's unit tests with the same settings and the `test`
action. With the delivered arm64 libraries, explicitly set `ARCHS=arm64` and
`ONLY_ACTIVE_ARCH=YES`. A Release test invocation also needs
`ENABLE_TESTABILITY=YES`; keep retained release app bundles separate from test
hosts and their XCTest payload. Use T3 `device_list`, then `device_open`, and the command it returns for
all simulator interactions. tvOS must be exposed by T3 before its remote UI
journey can be accepted.

The Foundation-only playback lifecycle also has executable host checks. Build
`Sources/Models.swift`, `Sources/PlaybackLifecycle.swift`, and
`tools/LifecycleChecks.swift` together with `swiftc -parse-as-library`, placing
the executable and module cache in the bounded build cache. These exercise the
production lifecycle helper, including ordered teardown before logout,
cleanup after reporting failure, cancelled preparation, and ownership transfer.
They do not substitute for AVPlayer or simulator acceptance.

`tools/StoreLifecycleChecks.swift` separately compiles with those two production
sources plus `Sources/LibraryStore.swift` and `Sources/MusicLibraryModel.swift`.
It substitutes host-only network,
Keychain and player adapters while executing the real store dismissal and
sign-out methods. It holds an in-flight report, closes the player, signs out,
and verifies captured-account cleanup completes before logout. It also checks
that cleanup failure remains visible after dismissal. Compile each check as a
separate executable, since each defines its own entry point.
Run the store executable with `--vault-failure` to check that failed Keychain
deletion preserves the saved account, reports the failure and allows sign-out
to succeed on retry. Its Keychain adapter never accesses a real saved account.

## Development reuse

For release candidates, compile `tools/BuildCore.rs` with stable Rust into your
bounded build cache, then run it from this directory. Set `CARGO_TARGET_DIR` and
`BLUEFIN_CORE_OUT` to absolute cache/deliverable paths outside source. An optional
argument selects one of the four Apple targets. This helper selects the matching
Xcode SDK, keeps the OS 15 deployment floor and remaps source, home, toolchain and
cache paths in Rust output. Use `CARGO_ENCODED_RUSTFLAGS` for additional flags.
`CARGO_BUILD_JOBS` accepts 1 through 4 and defaults to 2. Use 1 on a
memory-constrained Mac; the same limit applies to the prerequisite checks.
Point Xcode's `BLUEFIN_RUST_LIB_DIR` at the matching retained candidate directory.
Audit the final bundle too; compiler path remapping is not a secret-data scan.

After generating the project, compile `tools/BuildApps.rs` into the build cache.
Set `BLUEFIN_XCODE_DERIVED`, `BLUEFIN_APP_OUT` and `BLUEFIN_CORE_OUT` to absolute
paths outside source, then run the helper from this directory. It builds the
four unsigned arm64 Release applications and retains bundles outside derived
data. Optional `ios-sim`, `ios`, `tvos-sim` or `tvos` selects one target. Existing
candidate bundles are preserved; choose a fresh output directory for a new set.
Use fresh derived data for retained application bundles too. The helper rejects
stale XCTest bundles and externally staged model test fixtures in app output.

`BuildCore.rs` first runs the [native dependency boundary check](../../gates/client-boundary/README.md).
Provider discovery adapters and their administration code belong to the server
and web administration application. Native clients may consume generic Bluefin
results/actions only when the server enables and authorizes them. No provider
connector, credential, mapping or direct request implementation belongs here.

Lustr was inspected at commit `240e08d91d4aed0299639f6f11fd522912b248f9`.
Its current compiler is a Linux x86_64 Rust subset with no async, unsafe, or
general proc-macro support. It cannot compile this client's FFI, Reqwest, or
Iroh dependency graph. Its own documentation reports slower generated program
execution. It is not used as an Apple release compiler or represented as a
measured speedup here. A future portable, dependency-free policy module can
be checked against Lustr only after a parity benchmark proves a useful loop.

Build paths, signing configuration and runtime server addresses are supplied by
the developer's environment. Keep deployment records outside the checkout.

## Upstream review

Ferrofin source baseline is `b7d124a22d49bacf19f6c3c940333f508d647fb6`.
The app uses the Jellyfin HTTP contract rather than linking server crates or
adopting a live database. The live deployment review is retained privately;
its local topology, user accounts, credentials, and catalog are not fixtures
in this repository.

These are feature references, not claims that their implementations were
copied or that Bluefin has reached parity:

| Reference | Useful behavior | Bluefin status |
| --- | --- | --- |
| [Swiftfin](https://github.com/jellyfin/Swiftfin) and its [player comparison](https://github.com/jellyfin/Swiftfin/blob/main/Documentation/players.md) | Native Apple navigation, AVPlayer integration, system AirPlay, frame-rate matching tradeoffs | Native shell implemented; physical playback/system-feature acceptance pending |
| [Streamyfin](https://github.com/streamyfin/streamyfin) | Downloads, intro/credits skipping, trickplay, Seerr requests | Verified audio-download implementation; intro/credits and trickplay remain gaps. Provider discovery stays server/web only; native clients consume authorized generic results. |
| [Moonfin](https://github.com/Moonfin-Client/Moonfin-Core) | Household profiles, discovery, synchronized settings, watch together | Reviewed; server-specific integration and authorization remain unqualified |
| [Finamp](https://github.com/jmshrv/finamp) | Gapless music, offline queue/progress, ReplayGain, CarPlay | Dedicated music, verified offline audio, queue preloading and metadata gain implemented. Acoustic gapless and provisioned CarPlay journeys remain unqualified. |
| [Apple HIG](https://developer.apple.com/design/human-interface-guidelines/) | Native navigation, focus, accessibility, motion and platform controls | System controls used; Dynamic Type, VoiceOver, remote focus and Reduce Motion require device QA |

The source includes Live TV and guide navigation, chapters, track/quality
renegotiation, music queue preloading, sports, trailers and authorized Sing
packages. Each still needs the relevant connected-server and device journeys.
Native offline HLS downloads, DVR management, trickplay, intro/credits skipping,
video episode autoplay, household switching and full preference synchronization
remain gaps. Premium parity and performance require measured acceptance.

Audio details use the negotiated source-track index, or an unambiguous server
default. An unknown selection never borrows the first track's quality badge.
Hi-Res Lossless describes known 24-bit-or-higher lossless source audio above
48 kHz. Server conversion and Sing processing are labeled separately; the
current output route, sample rate and channel count do not prove bit-perfect
or lossless wireless output.

## Private distribution

The requested destination is private TestFlight testing, with no public App
Store release or public tester link. A free Personal Team cannot use
TestFlight. Its on-device provisioning expires after seven days. A paid
Developer Program membership enables TestFlight, where each build expires
after 90 days and must be replaced with a newly uploaded build.

A remote Mac can build, sign, and upload replacements using supported Apple
tooling after signing access and distribution ownership are established.
No enrollment, payment, signing credential export, upload, tester invitation,
scheduled job, or automatic build renewal has been performed.

Bluefin is the intended organization brand. Apple requires a verified legal
entity and normally a D-U-N-S Number for organization enrollment. Individual
memberships use the person's legal name; changing the Apple Account display
name does not change the App Store seller name. One Apple Account may belong
to multiple teams. A registered trade name can be an organization's developer
display name, while its legal entity remains the seller. Do not invent legal
entity details or enter an alias as the account holder's legal name.

Sources: [account comparison](https://developer.apple.com/help/account/basics/about-your-developer-account),
[TestFlight overview](https://developer.apple.com/help/app-store-connect/test-a-beta-version/testflight-overview/),
[developer names](https://developer.apple.com/help/app-store-connect/create-an-app-record/set-your-developer-name),
[enrollment](https://developer.apple.com/help/account/membership/program-enrollment/).

## Output ownership

Keep source in a checkout, generated outputs in bounded external storage,
and current deliverables plus one useful rollback outside
target directories. After checking source and artifacts, delete owned compiler
intermediates and scratch outputs. Keep small logs, hashes, first failures,
and actual available-space observations. Preserve shared caches, held work,
user data, keys and installed services.
