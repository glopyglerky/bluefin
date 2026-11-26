# Music playback and materials

## Experience

Bluefin uses a smooth navy, silver, and lavender canvas. Repeating line textures are removed from the web and Apple app backgrounds. Glass is reserved for floating navigation and playback controls. Media artwork, lyrics, and library rows stay readable.

The main categories remain Home, Sports, TV Shows, Movies, Live TV, and Music. Existing account ownership, Home personalization, refined search, and instance connection boundaries are unchanged.

Apple clients use the public SwiftUI Liquid Glass API where the target OS supports it. Earlier systems retain material controls. Reduced transparency and reduced motion receive solid, restrained controls. This does not use a private Apple Music API.

## Playback and lyrics

The web player presents artwork, lyrics, and queue views alongside its persistent mini-player. Music library labels sit below album artwork. Artists use circular artwork and library sections use clear rows.

Both web and Apple lyrics follow the actual media playback clock. The authenticated server contract remains `GET /Audio/{itemId}/Lyrics`:

- Time fields use 100-nanosecond ticks.
- A positive metadata offset delays display.
- Cue start and end times are absolute song times.
- Cue positions are UTF-16 offsets, with an exclusive end position.
- A lyric line has a start time; no line end time is invented.
- Untimed lyrics remain readable.
- Word highlighting uses supplied word intervals. Missing or invalid intervals never become synthetic word timing.

A line can be selected to seek to its actual position. Paused native seeks publish the completed player position. Manual browsing suspends automatic following; Follow lyrics immediately returns to the current line, even while paused.

Requests are canceled or rejected when the account, song, playback generation, or active request changes. Missing lyrics, local files without lyrics, and network errors have explicit states and do not stop audio playback.

The server resolves stored metadata and application-data tokens through its existing live directory resolver before reading or deleting a lyric file. This fixes adopted library entries that advertise lyrics while their stored path remains virtual. No database migration or media rescan is required.

## Singing and the iPhone microphone

On a supported Apple TV, the route is:

1. Open Now Playing, then Lyrics and Sing.
2. Open Sing, then Connect iPhone microphone.
3. Select the phone with Apple's system Continuity picker.
4. Choose Start microphone explicitly.

Connection alone does not start capture. Mute, voice level, and Disconnect remain available. Capture ends on pause, song or account changes, interruptions, dismissal, and background transitions. The phone follows the system pairing experience. This is the public Continuity microphone route initiated inside Bluefin.

Ordinary synchronized lyrics and microphone controls do not depend on prepared karaoke stems. Vocal adjustment remains a separate server capability. It requires configured model assets and trusted timing for the exact song source. A disabled or unprepared server cannot honestly offer complete vocal separation.

## Verification boundaries

The repository includes lyric timing and microphone lifecycle checks and web regressions for account/request cancellation, offsets, cue boundaries, seeking, and readable untimed states. The self-contained music preview uses original synthetic text, synthetic artwork, and a silent virtual clock. It is a test fixture and must not ship in the production asset package.

Release evidence must separately record full iOS and tvOS builds, the exact server binary qualification, installed device versions, and real HTTP checks. Host checks do not establish microphone sound output, echo cancellation, or phone-to-TV latency. Those require supported physical devices and explicit microphone permission.

## Public API references

- [Adopting Liquid Glass](https://developer.apple.com/documentation/technologyoverviews/adopting-liquid-glass)
- [Applying Liquid Glass to custom views](https://developer.apple.com/documentation/swiftui/applying-liquid-glass-to-custom-views)
- [Materials design guidance](https://developer.apple.com/design/human-interface-guidelines/materials)
- [Supporting Continuity Camera in a tvOS app](https://developer.apple.com/documentation/avkit/supporting-continuity-camera-in-your-tvos-app)
- [Public Continuity microphone port](https://developer.apple.com/documentation/avfaudio/avaudiosession/port/continuitymicrophone)
