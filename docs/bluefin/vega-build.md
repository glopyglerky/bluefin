# Vega native build

The development toolchain uses official Vega CLI 1.4.9 and SDK/VVD 2.0.13986.
The SDK-generated React Native 0.83 template supplies the configuration,
with `react-native-kepler` 4.0 and `react-native-w3cmedia` 2.3.5. The lockfile
pins the dependency tree. Install the SDK through Amazon's official process
and review its applicable license. SDK implementation, system bundles,
proprietary assets, signing profiles and generated packages stay outside Git.

From `apps/bluefin-smarttv/vega`, with the licensed CLI on PATH and an external
build directory selected:

```text
npm ci --ignore-scripts --cache <generated-output>/npm-cache
vega project doctor
npx tsc --noEmit
node --test session.test.mjs media-policy.test.mjs
CMAKE_BUILD_PARALLEL_LEVEL=2 npx react-native build-vega --build-type Release --target aarch64 --max-workers 2 --additional-native-options="--no-profile"
vega exec vpt validate build/aarch64-release/bluefinvega_aarch64.vpkg
```

The React Native release build bundles JavaScript and Hermes into the vpkg.
`vega build` alone is not sufficient to establish that the app is bundled.
`--no-profile` creates a development package; it does not establish signing,
installation eligibility, certification or store readiness.

`HlsPlayer.ts` adapts Apache-2.0 hls.js 1.7.4 to public Vega MediaSource,
SourceBuffer and native-surface APIs. Keep the dependency's license. Workers
and progressive fetching are disabled. Requests are restricted to the exact
opaque media grant. Foreign grants, queries, fragments, reported redirects
and encrypted playlists are rejected. Response size is checked after reading,
so this loader does not provide a hard streaming-memory limit. Fetch redirect
behavior, MSE adaptation, captions and real playback require native testing.

The shell uses standard MediaBrowser Authorization tokens, viewer-scoped
catalog requests, memory-only credentials and captured playback-release
ownership. Queue load/save uses account-scoped revisions. Source audio and
subtitle choices require a new lease with the selected indices and position.
The guide can display programme data, but does not admit live sources.
Automatic queue advance, shuffle/repeat and caption rendering remain unqualified.

The candidate targets Vega OS 2.0. Earlier development SDKs do not establish
previous-GA runtime support. Validate actual native output, media focus,
remote navigation, Back, suspend/resume, account changes and cleanup on each
claimed platform. A successful build or package validation is not a runtime
result. Retain machine-specific logs, package hashes and execution evidence
privately outside the source tree.
