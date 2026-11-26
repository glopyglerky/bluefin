# Bluefin in Safari on Apple HomeView

The default browser presentation is semantic HTML built by Rust/WASM. It does
not initialize eframe, WebGPU or WebGL. Native desktop keeps its existing egui
presentation. Browsers can explicitly request `?presentation=canvas` as a
presentation preference; this is never selected by user agent or device claims.

The browser uses Bluefin's existing controller, protocol, account epoch and
Service lifecycle. Library, series/album children, Music, Live TV, Search,
Queue, Account and generic server discovery forms use the same commands and
capability checks. No Seerr connector is included. Native labeled inputs,
buttons, links, headings, navigation and media controls expose real actions.
The page scrolls in normal document flow with no fixed navigation or modal.
Focus indicators and 44 CSS pixel controls support keyboard and touch use.
Editing fields are not replaced by asynchronous updates; other refreshes
restore surviving focused controls. Short, square and landscape screens are
supported by fluid layouts, wrapping and scrolling.

Playback remains authenticated by the existing ephemeral media worker and
Rust lifecycle. Native controls allow user-gesture playback, pause and seek;
video uses `playsinline`, including when the selected item is audio. Native
HLS is preferred when the media element reports it. Existing HLS MediaSource
support is used only when available. Browser direct-play profiles probe
`canPlayType` for container/codec pairs; these hints are not a guarantee of
decoding a particular file. Decode and negotiation failures remain visible.
No hidden stream fallback or protected provider extraction is performed.
Stop and sign out retain worker cleanup and server revocation acknowledgement.
Tokens and capability URLs are not written to local/session storage or app
markup. Native media source receives only the existing ephemeral worker route.

Audio and subtitle stream selection remains server negotiated. The current
Track DTO supplies index/type/title/language/codec, with no authenticated,
validated WebVTT delivery URL. Consequently no HTML track URL is fabricated;
native external captions await that contract. Existing server encoded subtitle
selection remains available. This is not an assertion of WebVTT support on a
particular device.

Primary references read November 15, 2026:

- [WebKit: Safari on Apple HomeView](https://webkit.org/blog/18608/safari-on-apple-homeview/)
  (November 13): iOS user agent, feature detection, voice-operable semantic
  controls, square/short screens, recommended 448×364 responsive test size.
- [Apple Developer: CarPlay](https://developer.apple.com/carplay/): native
  CarPlay audio/video app support on HomeView is a distinct delivery route.
  This web implementation neither enrolls nor installs a native app.
- [Apple Support: HomeView updates](https://support.apple.com/en-us/127196):
  Safari and third-party entertainment availability in HomeView software.

Chromium fixture/layout checks establish only browser behavior in that test
environment. Actual HomeView hardware, Safari, Siri, screen reader, native IME,
production account/CORS and codec/device acceptance require separate checks.
