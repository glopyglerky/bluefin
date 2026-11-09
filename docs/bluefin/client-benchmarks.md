# Native client comparison

Use actual built Bluefin screens beside the publishers' current screenshots.
Compare the same task and device class. Marketing images and feature listings
are references, not evidence that we tested another app or matched its behavior.
Reference artwork and screenshots stay outside this repository and app bundles.

## Reference applications

- [Manet Music](https://apps.apple.com/us/app/manet-music/id6470928235) and
  [Manet Music Player](https://apps.apple.com/us/app/manet-music-player/id6829167614):
  native music browsing, album and track hierarchy, compact controls and focused
  Now Playing. These are distinct listings; record which version was reviewed.
- [Infuse](https://firecore.com/infuse): artwork-led video browsing, media details,
  track/subtitle/chapter controls, playback continuity and format compatibility.
- [Plex](https://apps.apple.com/us/app/plex-find-movies-and-tv-shows/id383457673):
  library navigation, search, watched/resume states and remote-friendly discovery.
- [Plexamp](https://plexamp.com/): artwork-led music playback, queue editing,
  offline use, loudness controls and uninterrupted listening.

Use Apple Music and Apple's current platform guidance for native interaction.
Do not copy another application's branding, assets, proprietary implementation
or paid-service access. Bluefin's goal is comparable task quality with its own
design and supported integrations. Purchasing or installing competitors is not
part of this review.

## Acceptance comparison

| Area | Standard to meet | Bluefin status and remaining proof |
| --- | --- | --- |
| Library and music hierarchy | Artwork first, legible native text, clear sections, short paths to play | Native shelves, grids, album/artist/playlist pages and original preview artwork. Compare real populated libraries, long titles and loading/error states. |
| Navigation | Expected back behavior, a persistent selection, adaptive tabs/sidebar and keyboard access | Modern navigation stacks and adaptive native tabs with older-OS fallback. Verify actual iPad resizing, selection and keyboard journeys. |
| Mini-player | Persistent track context and accessible playback without covering content | Native tab accessory on supported iOS, safe-area fallback elsewhere. Verify inline/expanded placement and output routes. |
| Now Playing | Large artwork, readable metadata, reachable controls and predictable queue access | Width-based layout, native output volume/route controls, queue, favorites, lyrics and audio settings. Narrow-window regression has a native test. |
| iPad and Duo | Use available space, retain playback and navigation during resize, respect safe areas | No screen-size/phone-idiom layout assumptions. Native viewport fixtures cover compact and expanded sizes. Actual Duo poses require the matching runtime and SDK. |
| Apple TV | Readable ten-foot layout, clear focus, predictable Back/Menu and reachable controls | Native tvOS app and app-hosted playback tests. Runtime playback checks and manual remote/focus/accessibility acceptance are separate gates. |
| Video formats | Correct source/track/quality negotiation and truthful output support | Native AVPlayer and authorized server conversion, chapters and track/quality controls. Current admission profile is conservative SDR; broad HEVC/HDR/Dolby parity is not established. |
| Continuity | Correct resume, start/progress/stop and next-item behavior | Lifecycle and timeline handling tested with fixtures. Real episodes/up-next, server interruption and route changes still need end-to-end acceptance. |
| Audio quality | Reliable direct lossless playback and continuous transitions | ALAC direct/offline fixture, next-item preload, six-band DSP and normalization. Acoustic gapless, bit-perfect output and high-end external routes are not established by source or simulator tests. |
| Offline | Authorized download, integrity, cancellation, removal and usable storage controls | Native verified audio downloads and local playback tested. Full-library scale, resumption, offline preloading and progress reconciliation remain gaps. |
| Rich discovery | Useful personalized mixes, waveform/analysis and library exploration | Do not claim Plexamp sonic/radio/waveform parity. These need their own lawful metadata/analysis implementation and acceptance. |
| Ecosystem | Native Siri, sharing, CarPlay, SharePlay and home interactions | Guarded integration source exists. Provisioning, entitlement, partner-API and real-device journeys remain separate gates. |
| Performance | Fast launch, smooth scrolling, bounded artwork/memory and prompt first media | Bounded caches and lightweight native builds. Record measured release performance on populated libraries; app size or a screenshot does not prove speed. |

## Review procedure

1. Record source, app build and reference version. Use original synthetic preview
   content or an explicitly authorized test library, with no private data in
   published artifacts.
2. Capture Home, library, album/details, Now Playing and queue at comparable
   sizes. Include light/dark appearance, long titles, Dynamic Type and empty/error
   states. Identify differences in density, spacing, hierarchy and control reach.
3. Exercise the tasks behind the controls. A visible download, favorite, source
   selector or provider button must not imply untested access or working behavior.
4. Test compact/expanded resizing without restarting playback or losing queue
   position. Separate viewport fixture results from actual Duo fold/pose tests.
5. Test TV focus and remote navigation independently of native playback fixtures.
   A successful TV compile is not a TV runtime pass.
6. Keep unresolved differences explicit. Do not mark overall premium parity
   while format, continuity, offline, ecosystem or performance gates remain open.

Apple references: [Duo preparation](https://developer.apple.com/iphone-duo/prepare/),
[layout](https://developer.apple.com/design/human-interface-guidelines/layout),
[adaptive tab navigation](https://developer.apple.com/documentation/swiftui/enhancing-your-app-content-with-tab-navigation),
[state-preserving layouts](https://developer.apple.com/documentation/swiftui/anylayout).
