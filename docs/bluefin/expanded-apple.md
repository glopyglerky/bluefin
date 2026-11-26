# Expanded Apple targets

The first-release scope includes Mac, Apple Vision Pro and Apple Watch. New
platforms support OS majors 27 and 26 only. Existing iOS/tvOS support and prior
qualification are preserved. This work adds no older-OS test matrix.

## Targets and artifacts

| Scheme | Native destination | Minimum OS | Required Rust archive |
| --- | --- | --- | --- |
| BluefinMac | Mac Catalyst, Apple silicon | macOS 26 | aarch64-apple-ios-macabi |
| BluefinMac | Mac Catalyst, Intel | macOS 26 only | x86_64-apple-ios-macabi |
| BluefinVision | visionOS device | visionOS 26 | aarch64-apple-visionos |
| BluefinVision | visionOS simulator, arm64 | visionOS 26 | aarch64-apple-visionos-sim |
| BluefinWatch | watchOS device | watchOS 26 | None |
| BluefinWatch | watchOS simulator, arm64 | watchOS 26 | None |

Mac reuses the real SwiftUI library, Keychain and AVKit player through Catalyst.
It has a minimum window content size, standard resizable windows and Playback
menu commands for Space, Command-period and Command-R. Space is subject to
normal focused-control keyboard handling. Intel packaging targets macOS 26.
The selected official SDK must support x86_64 Catalyst at that deployment
target; a newer SDK version alone does not establish incompatibility. Compiler
and linker checks qualify the build separately from execution on Intel hardware.
Do not label an Intel build as macOS 27 qualified.

The Mac target explicitly enables App Sandbox and Hardened Runtime. Its
entitlements allow outgoing server connections, incoming connections for the
local playback bridge and peer transport, and microphone input for Sing.
Microphone capture still requires the viewer's permission. The network
entitlements do not replace playback authorization or the bridge's loopback
and capability checks. Offline files stay in the app's container; the target
does not request unrestricted access to the user's files.

SharePlay uses the same group-session entitlement as the iOS/tvOS targets.
Local ad hoc signing can verify the packaged entitlement values, but does not
establish distribution provisioning, connected SharePlay or microphone
acceptance. Validate the final signed app in its sandbox before distribution.

Vision links against the visionOS SDK with device family 7. Its native resizable
window defaults to 1100 by 760 points and retains SwiftUI navigation and AVKit
transport controls. Unsupported external-playback properties are excluded.
No immersive environment, stereo video, HDR, Iroh or hardware audio acceptance
is established by target creation or typechecking.

## Independent Watch app

BluefinWatch is a watch-only independent app, not an embedded iPhone companion.
Device packages contain both arm64 and arm64_32 slices for the selected
watchOS deployment floor. The simulator package uses the host's arm64 slice.
The viewer enters an HTTPS server address and standard account credentials on
Watch. The session lives in a device-only Keychain item. Passwords are discarded
after sign-in. There is no WatchConnectivity token transfer, credential deep
link, iPhone remote-control contract or plaintext credential log.

The small Swift adapter calls the existing Rust server's authenticated contracts.
It displays up to 50 audio items, favorites and the first 50 entries of the
server queue, preserving queue order. It offers play/pause, previous/next within
the displayed collection, favorite changes, stop and sign-out. Queue changes
are not written back automatically. It requires Bluefin opaque audio leases
at `/Bluefin/TV/v1/leases`; a server without that extension cannot play music
on this target. The server retains viewer authorization for each media request.
No private AVURLAsset header option is used. Capability paths remain private
runtime state and are never shared as user-visible links.

Playback uses AVPlayer and a long-form AVAudioSession activated asynchronously.
Available audio routes depend on Watch hardware and the OS. The system Now
Playing view supplies native route and volume controls. There is no Watch video,
HDR, Sing, download, background refresh or Iroh implementation. Lease cleanup
runs on stop, replacement, end and sign-out. Failed cleanup remains retryable
and blocks account removal or replacement playback. The server also expires
leases. Continuous unattended playback, interruption recovery and real hardware
route behavior still require acceptance. Signing/provisioning and independent
installation require the appropriate Apple account and distribution setup;
none is performed by the build helper.

Watch control requests send the standard MediaBrowser Authorization token,
refuse redirects, cancel responses above 2 MB while reading and apply a
30-second total resource deadline. Playback cleanup joins concurrent Stop
callers for the same captured lease and account. Delayed item callbacks cannot
stop a replacement item. Failed local credential removal keeps the account
visible with a retry message. These source and host checks do not establish
Watch device or connected-server acceptance.

For a host-only cleanup retry regression, compile `Watch/WatchAPI.swift`,
`Watch/WatchLeaseLifecycle.swift` and `tools/WatchLeaseChecks.swift` together
with `swiftc -parse-as-library` from the Apple app directory. The executable
uses synthetic accounts and delayed release callbacks. It does not contact
a server or access Keychain. It checks that a late waiter from a failed Stop
attempt cannot clear a later retry for the same lease.

## Build

From `apps/bluefin-apple`, run `xcodegen generate`. Compile
`tools/BuildExpandedApple.rs` with stable `rustc --edition=2021` into a bounded
external cache. Set absolute `BLUEFIN_XCODE_DERIVED` and `BLUEFIN_APP_OUT`
locations outside source, then run the helper with exactly one selection:
`mac-arm64`, `mac-intel26`, `vision`, `vision-sim`, `watch` or `watch-sim`.
Use fresh roots for each candidate. The helper uses one Xcode job and retains
unsigned application bundles outside DerivedData. It never invokes Cargo.

For Mac and Vision, set `BLUEFIN_CORE_OUT` to retained archives organized under
the exact triples above. Existing iOS/tvOS archives cannot substitute. The
build owner must qualify each core archive separately. `BuildCore.rs` accepts
the expanded triples explicitly; its default selection remains the four
iOS/tvOS targets. Rust lists the Catalyst and
visionOS targets in its [platform support](https://doc.rust-lang.org/rustc/platform-support.html).
A supported target does not prove this dependency graph compiles for it.

## Qualification boundaries

The [Apple launch support requirements](apple-support.md) cover recent device
models, HomeView's supported integration paths and the distinction between
hardware coverage and operating-system qualification.

Real 27.0 SDK source typechecks cover Mac arm64, Vision device and Watch device
with deployment floor 26. Simulator or device runtime journeys, connected-server
authentication, revoked access, audio routes, window/keyboard behavior and
accessibility remain separate gates. Watch and Vision simulator runtimes must
be installed before simulator execution. No existing device session should be
acquired to satisfy these gates. Keep generated output on bounded external build
storage; retain bundles, hashes and concise evidence on durable build storage.
Delete owned intermediates after checkpointing, preserving source and rollback.

Primary Apple references:

- [Configuring multiplatform destinations](https://developer.apple.com/documentation/xcode/configuring-a-multiplatform-app-target)
- [Independent watchOS apps](https://developer.apple.com/documentation/watchos-apps/creating-independent-watchos-apps)
- [Watch-only bundle key](https://developer.apple.com/documentation/bundleresources/information-property-list/wkwatchonly)
- [Watch audio activation and route requirements](https://developer.apple.com/documentation/avfaudio/avaudiosession/activate(options:completionhandler:))
- [Bringing apps to visionOS](https://developer.apple.com/documentation/visionos/bringing-your-app-to-visionos)
