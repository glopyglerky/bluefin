# Vizio and Vega clients

These are development candidates. Vendor installation, certification and device
acceptance remain required. Release support targets current and immediately
previous GA operating-system generations. Android-based Fire TV, Google TV and
Roku results do not establish Vega or Vizio compatibility.

## Vizio

The hosted candidate reuses the common web client and Rust/WASM policy, with
`vizio/platform.js` supplying feature detection and media adaptation.
`vizio/stage.rs` assembles a fresh hosted directory and refuses overwrite.
HTML audio and HLS are available only when the runtime reports support.
Opaque HTTPS playback routes, bounded seek, exposed audio/text tracks, native
errors and source removal on stop/pagehide use standard browser interfaces.
The shared client supplies remote geometry, viewer-scoped browsing, memory-only
tokens and lease/report/release. Pagehide release is best effort; server lease
expiry remains necessary.

This adapter does not invent Vizio lifecycle, launch, key-registration or exit
APIs. Obtain the approved architecture, hosting/origin requirements, remote and
lifecycle APIs, current/previous firmware matrix and certification process
through [Vizio's content partner programme](https://www.vizio.com/en/content-partners).
Its [developer specifications](https://docs.developer.vizio.com/) require partner
access. A staged directory or ordinary-browser test is not a native Vizio
installation or vendor approval.

## Vega

The React Native shell uses public Amazon W3C media and Kepler interfaces,
React Native AppState and BackHandler. It provides authenticated library,
music, search, folder and detail browsing, queue operations, guide display,
native-surface playback controls, progress reporting and background cleanup.
Tokens remain in memory; player URLs use opaque server grants. No provider
credential, DRM extraction or acquisition integration belongs in this client.

The hls.js adapter uses public MSE interfaces. This is implemented source;
HLS/live playback, native captions, HDR, surround formats, media controls and
remote/lifecycle behavior still require native runtime qualification. The shell
uses React Native, not a Rust-native Vega binding. See the
[Vega build instructions](vega-build.md) for the dependency and package path.

Use official licensed tooling and an authorized developer runtime or device.
Follow the vendor's signing, enrollment and certification process before
installation or distribution. Supported OS/device versions must be verified
against [Amazon's release notes](https://developer.amazon.com/docs/vega/2.0/vega-release-notes),
not inferred from an experimental SDK version. Relevant public references are
[SDK installation](https://developer.amazon.com/docs/vega/2.0/install-vega-sdk),
[media setup](https://developer.amazon.com/docs/vega/2.0/media-player-setup) and
[library porting](https://developer.amazon.com/docs/vega-api/2.0/porting-libraries).

## Focused checks

Use an external generated-output directory and bounded compiler parallelism:

```text
node --test apps/bluefin-smarttv/vizio/platform.test.mjs apps/bluefin-smarttv/vega/session.test.mjs apps/bluefin-smarttv/vega/media-policy.test.mjs
rustc --edition=2024 -C opt-level=2 -C debuginfo=0 apps/bluefin-smarttv/vizio/stage.rs -o <output>/bluefin-vizio-stage
<output>/bluefin-vizio-stage apps/bluefin-smarttv <qualified-wasm> <fresh-hosted-directory>
```

Synthetic auth, grant, cancellation and player-adapter tests are distinct from
SDK compilation and native playback. Qualify authorization/revocation, media
rendering, seek, track selection, remote focus, Back, suspend/resume and release
on each claimed vendor/model family. Keep actual test identities, machine
configuration, captures and package receipts in private execution records.
