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
action. Use T3 `device_list`, then `device_open`, and the command it returns for
all simulator interactions. tvOS must be exposed by T3 before its remote UI
journey can be accepted.

The Foundation-only playback lifecycle also has executable host checks. Build
`Sources/Models.swift`, `Sources/PlaybackLifecycle.swift`, and
`tools/LifecycleChecks.swift` together with `swiftc -parse-as-library`, placing
the executable and module cache in the bounded build cache. These exercise the
production lifecycle helper, including ordered teardown before logout,
cleanup after reporting failure, cancelled preparation, and ownership transfer.
They do not substitute for AVPlayer or simulator acceptance.

## Development reuse

Lustr was inspected at commit `240e08d91d4aed0299639f6f11fd522912b248f9`.
Its current compiler is a Linux x86_64 Rust subset with no async, unsafe, or
general proc-macro support. It cannot compile this client's FFI, Reqwest, or
Iroh dependency graph. Its own documentation reports slower generated program
execution. It is not used as an Apple release compiler or represented as a
measured speedup here. A future portable, dependency-free policy module can
be checked against Lustr only after a parity benchmark proves a useful loop.

The existing private infrastructure supplied a shared bounded build cache,
NAS compilation route, T3 device integration, and deployment observations.
Those private details are kept out of the public app. The inspected workspace
gateway's MCP ALPN is not a media protocol and is not used to bypass media
authorization.

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
| [Streamyfin](https://github.com/streamyfin/streamyfin) | Downloads, intro/credits skipping, trickplay, Seerr requests | Reviewed; not implemented or advertised as available |
| [Moonfin](https://github.com/Moonfin-Client/Moonfin-Core) | Household profiles, discovery, synchronized settings, watch together | Reviewed; server-specific integration and authorization remain unqualified |
| [Finamp](https://github.com/jmshrv/finamp) | Gapless music, offline queue/progress, ReplayGain, CarPlay | Reviewed; dedicated music/offline/CarPlay journeys remain unqualified |
| [Apple HIG](https://developer.apple.com/design/human-interface-guidelines/) | Native navigation, focus, accessibility, motion and platform controls | System controls used; Dynamic Type, VoiceOver, remote focus and Reduce Motion require device QA |

No claim is made to have audited every streaming client, copied proprietary
clients, or combined all upstream features. Native offline HLS downloads,
Live TV/DVR, chapters/trickplay, track switching requiring renegotiation,
up-next autoplay, media requests, household switching, preferences sync, and
performance budgets remain explicit qualification work.

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

Keep source under `~/repo`, generated outputs on the bounded Mac cache or NAS
build filesystem, and current deliverables plus one useful rollback outside
target directories. After checking source and artifacts, delete owned compiler
intermediates and scratch outputs. Keep small logs, hashes, first failures,
and actual available-space observations. Preserve shared caches, held work,
user data, keys, installed services, and the reserved NAS disks.
