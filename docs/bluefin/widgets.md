# Native widgets

Bluefin's WidgetKit lane provides generic quick access on iPhone and iPad. The
extension has an iOS 26 floor for the current and preceding major OS releases.
It contains no networking, credentials, media identifiers, artwork or playback
commands. The iPhone app embeds the extension and registers its navigation URLs.
The integrated simulator build passes. Native widget delivery and device
qualification remain unverified.

## Layout and navigation

Small widgets open Library. Medium and large widgets expose Library, Music,
Live TV and Search links. Circular, rectangular and inline accessory widgets
open Library. The widget uses native text styles, system symbols and container
backgrounds. Text participates in system redaction. Layout, localization,
Dynamic Type and VoiceOver still need device review.

Links use only `bluefin-widget://open/library`, `/music`, `/liveTV` or `/search`.
The scheme and host are repeated for each complete URL. `WidgetRoute` accepts
only exact canonical URLs. Queries, fragments, credentials, encoded paths,
ports, media identifiers and arbitrary actions are rejected. The app must check
its live session and preview state immediately before selecting the matching
tab. A signed-out request is dropped, with no deferred intent stored across
sign-in. Existing LibraryStore account authority remains responsible for all
subsequent data access. Opening a tab never starts playback.

These controls use `Link` and `widgetURL`, as Apple recommends for opening an
app. There is no widget AppIntent, button, toggle or AudioPlaybackIntent to
admit. The focused tests exercise the actual shared URL admission function,
including signed-out and preview rejection. They do not establish invocation
of a system intent or successful navigation in a running app.

## Snapshot contract

`WidgetSnapshot` schema 1 permits only signed-out, unavailable, stale or current
status, capture and expiry times, a random account-generation fence, and a false
personal-content opt-in flag. A generation is not a user or device identifier.
The parser rejects additional keys and enabled opt-in. Personal snapshots are
not supported, even after consent. Adding them requires a separate reviewed
shared-display design, explicit viewer consent, transition and failure handling,
and physical privacy qualification. SwiftUI redaction alone cannot provide that
guarantee.

Snapshots have a 1,024-byte bound and a maximum 15-minute lifetime. Unknown
schemas, malformed data, future captures, invalid lifetimes, mismatched expected
generations and expired snapshots are rejected. An otherwise valid expired
snapshot produces only a generic stale label. The provider schedules a stale
entry at the exact expiry. WidgetKit controls actual redraw timing. A timeline
reload request is not proof that an old rendered view disappeared immediately.

`WidgetSnapshotFile` resolves only an explicitly configured, entitled App Group
container. The `BluefinWidgetAppGroup` Info key is deliberately absent by default.
There is no dummy group, UserDefaults fallback, shared credential store or
authenticated extension networking. Without a container, the extension displays
unavailable and still offers generic links. This candidate configures no App
Group or signing entitlement and makes no free-account or TestFlight claim.

The single app-process writer serializes transitions and publication. Before an
account transition, it invalidates the previous generation, removes the old
file and writes a generic replacement. Publication requires the current healthy
generation. Failed removal or replacement blocks publication until a successful
transition. The extension is read-only. Use one writer instance, fresh random
generations per account lifetime, and await transition completion before new
status publication. Never derive authority from the snapshot or allow it to
select an account.

Writes use Foundation atomic replacement and complete file protection on iOS.
Reads use a bounded file handle and return unavailable on failure, including
protected-data unavailability. If both replacement and cleanup fail, an older
generic status can remain until expiry. Already rendered widgets can also
persist under system scheduling. No personal data exists in this schema, so
these limitations cannot expose titles from a previous account. Status freshness
is not an account privacy guarantee.

## HomeView and other hosts

Apple documents iPhone widgets on HomeView and supplies Safari there. This lane
provides an ordinary iPhone WidgetKit extension, not a standalone HomeView app
or download. It does not infer that HomeView can execute iPhone scenes. Generic
status is suitable for a shared display, but native tab navigation remains
unqualified on HomeView. Follow the displayed instruction to open Bluefin on
the iPhone when the host cannot launch the native scene. There is no fallback
that sends a server URL or account data to another host.

The small widget is marked disfavored in CarPlay because its native navigation
requires the iPhone scene. This is a system preference, not an enforcement or
privacy boundary. Audio/video CarPlay apps are separate integrations with their
own entitlement and provisioning requirements. No such approval is implied.

Watch complications are deferred. Reusing an iPhone-only scheme without a
qualified Watch navigation handler would create a broken control. The accessory
families here belong to the iPhone extension, not a new Watch extension. Safari
layout evidence, SDK builds, projected widgets and physical HomeView acceptance
remain separate results.

## Integration and qualification

1. Add an iOS app-extension target for `Widgets`, excluding `Tests` and the plist
   from compilation. Include `PrivacyInfo.xcprivacy` as a resource and use
   `Info.plist` for the WidgetKit extension point. Keep the 26.0 floor.
2. Reset inherited Rust bridging headers and linker flags for the extension.
   Embed it in the iPhone app. Compile `WidgetRoute.swift` in the app as well.
3. Register the `bluefin-widget` URL scheme. Route admitted URLs synchronously
   in `LibraryTabs.onOpenURL` to its existing tab values. Review cold start,
   signed-out launch, preview, account switching and sign-out during navigation.
4. Leave snapshot sharing unavailable until a real App Group is configured by
   an authorized signing owner. If enabled later, integrate the writer before
   account transitions and publish only after live account checks. Request a
   WidgetCenter reload after persistence. Do not report immediate redraw.
5. Audit the extension's final privacy manifest against linked code and resource
   contents. It currently collects no data and declares no required-reason API
   use. Keep host legal notices accessible and include any notices required by
   the final extension's actual dependencies. This lane adds no third-party
   library. Legal, agreement and distribution acceptance remain outstanding.
6. Build the integrated app after concurrent qualification releases ownership.
   Qualify all families on OS 26 and 27, locked and unlocked behavior, URLs,
   accessibility, and any offered projection on the actual host.

The app project includes the target, embedding, and URL routing in steps 1
through 3. App Group status sharing and device qualification remain separate.

Run the focused tests from the repository root with an Apple SDK toolchain,
placing the binary and compiler cache on the designated build filesystem:

```sh
swiftc -module-cache-path "$TMPDIR/widget-modules" \
  apps/bluefin-apple/Widgets/WidgetSnapshot.swift \
  apps/bluefin-apple/Widgets/WidgetRoute.swift \
  apps/bluefin-apple/Widgets/Tests/WidgetContractTests.swift \
  -o "$TMPDIR/widget-contract-tests"
"$TMPDIR/widget-contract-tests"
```

## Primary references

- [Creating a widget extension](https://developer.apple.com/documentation/widgetkit/creating-a-widget-extension)
- [Linking to app scenes](https://developer.apple.com/documentation/widgetkit/linking-to-specific-app-scenes-from-your-widget-or-live-activity)
- [Widget interactivity](https://developer.apple.com/documentation/widgetkit/adding-interactivity-to-widgets-and-live-activities)
- [Foundation atomic writing](https://developer.apple.com/documentation/foundation/nsdata/writingoptions/atomic)
- [Complete file protection](https://developer.apple.com/documentation/foundation/nsdata/writingoptions/completefileprotection)
- [App Group container lookup](https://developer.apple.com/documentation/foundation/filemanager/containerurl(forsecurityapplicationgroupidentifier:))
- [Safari on Apple HomeView](https://webkit.org/blog/18608/safari-on-apple-homeview/)
- [CarPlay and HomeView](https://developer.apple.com/carplay/)
- [Agreement changes covering iOS widgets on HomeView](https://developer.apple.com/news/?id=17mwd36z)
