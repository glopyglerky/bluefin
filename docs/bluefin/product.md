# Bluefin native client and server

Bluefin owns the client and server contracts. The goal is a single coherent
media system with native Apple navigation and playback, Rust protocol and
storage logic, and authenticated direct Iroh transport. Keep compatible
Jellyfin routes for existing libraries and clients. Negotiate Bluefin extensions
explicitly, using an authenticated versioned capability response. Transport
identity never replaces the viewer's media authorization.

## Design references

[Manet](https://tilosoftware.io/manet/) is the principal music reference. Its
published screens use album artwork, native typography, restrained controls,
a compact player, and a focused full-screen player. Its documented features
include offline music, lyrics, Siri, CarPlay and normalization. Manet+ adds
library selection, ignored tracks, a six-band equalizer and alternate icons.
Bluefin should provide useful equivalents without adding a paywall here.

[Infuse](https://firecore.com/infuse) is a video reference for library
organization, playback continuity, track and chapter controls, subtitles,
format negotiation, downloads and device synchronization. Its paid features
include additional formats, lossless audio, supported Dolby playback and sync.
[Prism](https://apps.apple.com/us/app/prism-music/id1335007451) is a reference
for music sorting/filtering, queue editing, playlists and offline use.

These are behavior and design references. Do not copy proprietary source,
branding, screenshots or album artwork into Bluefin. Reference screenshots
used for design review stay outside product source.

## Completion criteria

Source presence and successful compilation do not establish completion. Each
user journey needs a recorded build, actual device/server result and relevant
failure-path checks.

- Native iPhone, iPad and Apple TV browsing, focus, Dynamic Type, VoiceOver,
  Reduce Motion, keyboard and orientation behavior.
- Account-scoped Home, library/search pagination, album/artist relationships,
  playlist create/edit/reorder/delete, favorites and library preferences.
- A persistent editable queue, repeat/shuffle, interruption/route recovery,
  Now Playing artwork, CarPlay controls, offline music, lyrics, normalization
  and a usable equalizer. Verify continuous audio instead of inferring gapless
  playback from a queue class.
- Native video playback, accurate resume/reporting, audio/subtitle selection,
  chapters, episodes/up-next, hardware-aware format and quality negotiation,
  AirPlay and Picture in Picture on supported hardware.
- Profile-aware live events with current/next programmes. Server authorization
  applies to catalog, playback, adoption and every transport. Hidden tiles are
  not an access boundary. A programme restriction must not grant an entire
  mixed-content channel.
- A current supported Iroh graph, pinned peers, viewer authorization, bounded
  streaming/backpressure, cancellation, reliable reconnect and Apple target
  qualification. Do not ship the legacy compatibility experiment.
- Native sharing, exact provider content links with system handoff/store
  fallback, Siri/Shortcuts, provisioned SharePlay, CarPlay audio/video and home
  integrations. Represent unavailable entitlements and partner APIs honestly.
- Real server acceptance without losing the installed deployment's essential
  authentication, catalog/adoption, HLS and guide behavior. Preserve its data
  and rollback. Qualify a replacement before touching the live service.
- Measured launch, browse/scroll, artwork, first audio/video, memory, CPU and
  transfer behavior. No performance or bit-perfect claim without measurement.

## Development boundaries

The baseline targets iOS/tvOS 15 and later, with guarded newer capabilities.
This is not universal support for obsolete hardware without those operating
systems. Keep native system components. Use Rust for portable policy, protocol,
transport and persistence; use Swift where Apple platform integration requires
it. Keep private deployment identifiers, credentials and family catalog data
outside product Git. No public app submission is authorized.
