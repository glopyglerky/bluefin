# Bluefin Android

The Android client is a separate native Kotlin/Compose application in `apps/bluefin-android`. It targets API 26 and newer with compile/target API 36. Phone and tablet layouts use an adaptive grid. Android TV, Google TV and Android-based Fire TV devices use the same application with a Leanback launcher entry, larger spacing and visible D-pad focus borders. Touch is optional. Fire OS variants without Android APK support are outside this target.

This is an implementation candidate. SDK/NDK packaging and native linking passed for a frozen source snapshot. The final Android 15 AOSP emulator runs passed eleven of fourteen instrumentation tests. Three failures share an offline catalog defect, so full runtime acceptance remains held. Phone emulator results do not establish tablet, TV, Fire TV, accessibility or hardware-output acceptance.

## Snapshot build evidence

A frozen source snapshot built all three JNI ABIs and produced a debug APK, instrumentation APK and unsigned release APK with API 36. Four Gradle unit tests, full lint and instrumentation compilation passed. The runtime candidate retained those exact JNI bytes. All six packaged native libraries (Bluefin JNI and AndroidX graphics-path for each ABI) passed 16 KiB ELF load alignment checks; both application APKs passed 16 KiB ZIP alignment. JNI symbols, packaged JNI byte equality, manifest metadata and the debug signature were checked. Artifact content scans found none of the private build path/host prefixes checked. This bounded scan is not a complete security or license audit.

The first release lint failure required explicit `includeSubdomains="false"` on the two loopback network-security entries. That resource fix passed the focused rerun and final assembly. Existing vendored netwatch deprecation warnings remain.

The supported device route booted an owned Android 15 AOSP emulator. Actual JNI loading and identity-override refusal passed. Media3 playback time, native next/previous queue commands, seek, eight seconds of playback after controller disconnection, background session state and explicit stop passed with synthetic audio. After the ancestor-walk repair, final-root and ancestor symlink refusal also passed. Three assertions still fail when a fresh cache has no catalog. The same directory reads an existing catalog successfully; the failure is in missing-file handling, and its exact Android errno was not measured.

Authenticated loopback HTTP fixtures exercised actual CoreClient, Keystore, Rust and Media3. Audio decoding, queue navigation and advance, seek, started/stopped reporting, seven-second server preparation, immediate Stop during preparation, captured-account cancellation and secure-store failure handling passed. Standalone ExoPlayer also rendered authenticated video frames through the Rust proxy and passed seek and listener shutdown checks. This video test does not qualify the production video screen. The download test failed before it could verify completion, network-off playback or removal.

Delayed preparation ran with a resumed Activity and bound service. It does not prove the foreground-service startup deadline or backgrounding during preparation. Debug instrumentation enables the core's existing local-HTTP opt-in only for a fixed loopback fixture; the release adapter does nothing and contains no fixture switch. Production HTTPS validation remains enabled. These are synthetic HTTP API results, not real-server or HTTPS acceptance. The emulator, panel, fixture and port forwarding were closed after testing. Offline completion, actual foreground-service startup, authenticated background transitions, Iroh, tablet/TV/Fire and physical output remain acceptance gates. Exact hashes, first failures and receipts stay outside product source.

## Implemented paths

- Library, movie, episode, music, Live TV channel and search navigation, bounded pages and explicit load-more.
- Detail, resume, series/library child browsing and current channel program guide requests.
- Local account-scoped queue with automatic audio advance and native MediaSession next/previous commands in MusicService. Queue state is ephemeral; mixed queues stop at the audio segment. Native queue commands passed instrumentation. Notification-button UI taps, headset controls and process-death playback resumption remain unqualified.
- Service-owned Media3 music playback, MediaSession notification and background lifetime. Returning to the library preserves music; explicit Stop joins its cleanup. Video uses native PlayerView controls, pauses on lifecycle stop and releases its resources when leaving playback. Starting video stops and joins music first. Audio focus, noisy-output pause and foreground-service declarations are present. Controller admission requires the app UID or a trusted controller, denies media replacement and withholds private item/timeline access from external controllers. Instrumentation checked same-UID command restrictions and the non-exported service; a separate attacker-app runtime test remains unrun.
- AES-GCM account encryption using Android Keystore, synchronous private-preference commits, disabled Android backups and no saved-state password persistence. Sign out before changing accounts. Sign-out joins music, clears account secrets and resets library/queue state; secure-store clear failure retains account state for retry.
- App-private, no-backup offline root passed to the existing Rust download manager. Audio download, status, cancel, removal and completed-file playback controls are implemented. Only core-verified local URLs are playable; removal stops consumers before joining the writer. Video downloads are not implemented.
- Native authenticated artwork, limited to two concurrent requests, 2 MiB encoded bytes and sampled bitmap decoding near 512 pixels. It refuses redirects and credential URLs, uses no disk cache and checks the captured account before display.
- Original synthetic catalog, geometric artwork and a three-second generated WAV. Only the synthetic song has demo media. Other demo records explicitly have no playable movie, episode or broadcast.

Optional, core-projected `ExternalUrls` open HTTPS provider pages through the native Android URL handler after rejecting credentials, query strings, fragments and non-default ports. They carry no account headers or entitlement claim. The current shared core does not project that field, so this control remains dormant until that projection is admitted. There is no provider entitlement or DRM claim, commercial media fixture, cast implementation or discovery integration. Generic server extensions are unused; they must remain optional if added later, with HTTP 404 treated as unavailable.

## Shared core admission

`rust` builds `libbluefin_android.so` and exports only the JNI JSON adapter. It depends on the existing `bluefin-apple-core` rlib and calls its public `call(&serde_json::Value)`. Sign-in, main API requests, playback planning/reporting, Iroh transport, loopback media proxy and download rules stay in that crate. Native artwork loading uses a separate bounded HTTPS request with the same standard MediaBrowser identity and captured account.

The shared core now has Android identity, conservative Media3 profile selection and scoped sign-out implementation. These remain subject to parent integration and runtime qualification. JNI requires an early, local `androidSupport` operation with this contract:

```json
{
  "schemaVersion": 1,
  "clientIdentity": "Bluefin Android",
  "media3Profiles": true,
  "scopedStorage": true,
  "signOutCleanup": true
}
```

This operation must precede core server URL parsing and perform no network request. These flags are evidence-backed admission requirements, not values to return merely to enable the UI. All JNI requests carry `clientPlatform: android`. The core must use that discriminator to select a truthful Android identity using Standard MediaBrowser Authorization, preserve token validation and URL/redirect restrictions, and admit only conservative Media3 formats for the actual device. Kotlin supplies `androidCapabilities` with observed decoder MIME types and false probe/HDR/passthrough qualification. Decoder presence alone does not prove resolution, codec profile, HDR, passthrough or an asset's playability. Do not reuse Apple's `videoCapabilities` interpretation as Android evidence.

The shared core's `androidSignOut` cancels and joins account downloads, purges owned partial/completed offline state, retires account stream resources, and attempts server logout; these guarantees need Android runtime qualification. It returns `localCleanupComplete`, `serverRevoked` and `playbackReleaseComplete` independently. The Android app clears secrets even if cleanup or server revocation cannot be confirmed and displays that limitation. It never deletes files behind a possible native writer. Existing Apple `logout` cancels an active download but does not prove a completed-cache purge, so it does not satisfy this contract.

Online playback accepts only the core's credential-free HTTP loopback proxy on `127.0.0.1`; offline playback accepts only its verified private file URL. Credentials never go into Media3 headers. The manifest permits cleartext only to localhost and `127.0.0.1`; server connections remain HTTPS. The app does not bypass native asset probe requirements. Settings expose an optional independently verified Iroh peer ID and numeric address. Selection is memory-only, account-scoped and passed to audio/video playback; failures do not silently select HTTPS. Peer discovery/trust enrollment is not implemented. Android Iroh runtime acceptance remains unrun.

## Build and integration

Use JDK 17 and Gradle 8.11.1 with SDK platform 36, build tools 35.0.0 and NDK r28c (28.2.13676358). AGP 8.10.1, Kotlin/Compose compiler 2.1.20, Compose BOM 2025.04.01 and Media3 1.6.1 are pinned. A Gradle installation is required; no unverified wrapper binary is included.

```sh
gradle --no-daemon --project-cache-dir "$BLUEFIN_CACHE/project-cache" \
  -PbluefinBuildDir="$BLUEFIN_CACHE/build" \
  -PbluefinJniDir="$BLUEFIN_CACHE/jniLibs" \
  :app:testDebugUnitTest :app:assembleDebug
```

Run from the Android tree with `GRADLE_USER_HOME` under bounded generated storage and `ANDROID_HOME` pointing at a properly licensed SDK. Build Rust on the approved build host, with the required Android Rust standard library and licensed NDK available. Build the JNI manifest using the platform linker for each target; copy only its final shared library into the corresponding ABI directory supplied with `bluefinJniDir`. No JNI library is silently built during Gradle configuration. The Gradle pre-build guard requires a nonempty JNI library for all three advertised ABIs; it refuses to package an incomplete candidate.

| Device target | Rust target / JNI directory |
| --- | --- |
| Android phone/tablet/TV ARM64 | `aarch64-linux-android` / `arm64-v8a` |
| Android-based Fire TV ARM32 | `armv7-linux-androideabi` / `armeabi-v7a` |
| Emulator / compatible x86 TV | `x86_64-linux-android` / `x86_64` |

Compile `tools/build-native.rs` with Rust on the Linux build host. Invoke it with the source root, NDK root, bounded Cargo target directory and final JNI directory. It builds all three targets with the API 26 linker, source path remapping and 16 KiB maximum page alignment. It requires the configured Cargo home and installed Rust target standard libraries. Keep the explicit Rustup home and disable automatic toolchain installation in managed environments. Compile `tools/verify-native.rs` and pass every final `.so` to check ELF load-segment alignment. Also check the final APK with `zipalign -c -P 16 -v 4`; ELF alignment alone does not establish ZIP alignment or runtime compatibility.

Keep exact source and artifact hashes around each native build. A snapshot build does not qualify later changes to shared core inputs. Inspect the final manifest with `aapt dump badging`, native dependencies with NDK `llvm-readelf`, signatures with `apksigner verify`, and the Gradle dependency report. Debug signing keys are private development artifacts. Release APKs remain unsigned until separately authorized signing.

API 36 addresses the current phone submission target requirement; this is no store-readiness claim. Publication, account enrollment and uploads require separate authorization.

Android netwatch route monitoring is a no-op, and netdev has an optional Android context path. Network changes and context initialization require runtime qualification.

Cuttlefish can use crosvm, a Rust virtualization backend. Its documented host setup installs system packages, changes groups/udev rules and reboots. Those changes must not be applied to a protected appliance as an incidental build step. Prefer it when the host and supported device route permit it. The official Android Emulator is a fallback, subject to host-library and supported T3 device discovery checks. Neither backend establishes complete Android platform memory safety. Use supported device sessions for UI checks; installing the SDK or detecting KVM is insufficient.

The packaging-boundary owner must register these additional paths without relaxing existing server-integration exclusions:

- Manifest: `apps/bluefin-android/rust/Cargo.toml`.
- Sources: `apps/bluefin-android/rust/src` and `apps/bluefin-android/app/src/main/java`.
- Packaging/configuration: Android Gradle files, manifest, XML resources, assets and eventual APK/JNI dependency/license inventories.
- Existing shared local dependency allowlist remains necessary for the Apple core, modern Iroh crate, its two vendored network crates and `bluefin-contract`.

Keep final native dependencies, licenses and transitive packages under the parent's boundary check. No server discovery service, acquisition automation, upstream secret or client identity bypass belongs in this package. Before acceptance, build every claimed ABI, verify APK native library contents and 16 KiB page compatibility where required, then use supported T3 Android device sessions to test phone/tablet rotation, TalkBack, TV D-pad focus/Back/media keys, authentication, seek/resume, queue advance, live tuning/cleanup, sign-out during downloads, account separation and failure paths. Store runtime evidence outside product source.

## API references

- [Media3 ExoPlayer](https://developer.android.com/media/media3/exoplayer/hello-world)
- [Android TV media apps](https://developer.android.com/media/implement/surfaces/android-tv)
- [Android Keystore](https://developer.android.com/privacy-and-security/keystore)
- [App-specific storage](https://developer.android.com/training/data-storage/app-specific)
- [AGP 8.10 requirements](https://developer.android.com/build/releases/agp-8-10-0-release-notes)
- [Google Play target API policy](https://support.google.com/googleplay/android-developer/answer/11926878)
- [16 KiB page sizes](https://developer.android.com/guide/practices/page-sizes)
- [Cuttlefish host setup](https://source.android.com/docs/devices/cuttlefish/get-started)
- [crosvm](https://github.com/google/crosvm)
