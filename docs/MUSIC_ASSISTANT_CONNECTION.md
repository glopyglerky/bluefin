# Music Assistant connection

Bluefin's Apple app can connect a signed-in Bluefin account to an existing Music Assistant account. Open Settings, then Music Assistant, enter that server's HTTPS address and a token from the Music Assistant profile, and choose a player.

The connection exposes bounded track-library pages, library search, available players, the first 50 active-queue entries, explicit Play or Add actions, and supported pause/resume/previous/next/stop controls. It controls Music Assistant's existing players. It does not create a new audio player, extract stream URLs, fetch provider credentials, copy service audio, or synchronize a new group.

## Account and transport boundaries

The app stores the connection in an account-specific, non-synchronizing, ThisDeviceOnly Keychain item available while the device is unlocked. The key is derived from the existing Bluefin server and user identity. The payload records that same owner. These credentials are independent of the Bluefin session and are not included in account exports, Watch synchronization, invitations, or model requests.

Opening the screen checks the current Music Assistant server and user before accepting a saved connection. Changing Bluefin accounts or leaving the screen cancels UI work, clears displayed results and active UI credentials, and fences late responses. An already-started call can retain its request until it returns, and an HTTP command already sent can still take effect; canceling the UI does not prove that a remote playback command stopped. A transport failure after a mutation asks the user to refresh the queue before retrying. There is no automatic playback retry.

HTTPS uses the existing validated Rust HTTP client. Redirects and proxy-environment inheritance are disabled. Local HTTP requires explicit consent and is limited by the existing local-address validation. This connection does not silently reuse Bluefin mesh credentials or claim the Music Assistant API is post-quantum. A separately authorized mesh service adapter remains future work.

Library, player and playback requests are authenticated as the configured Music Assistant user. The connector never supplies the API's user-impersonation argument. The Music Assistant server remains the owner of provider, player, and queue permissions. Bluefin displays only selected response fields and does not copy private device details, source settings, stream details, or raw server error bodies into its UI.

## Compatibility

On connection, Rust reads /info, /auth/me and /api-docs/commands.json. It checks the known command names, supplied parameter names, and additional required parameters before enabling controls. A version with an incompatible schema does not get an invented fallback command. Command availability does not grant permission; the server can reject an action for the current user.

HTTP POST /api returns the command result directly. This differs from Music Assistant's WebSocket result envelope.

| Bluefin action | Upstream command |
| --- | --- |
| Library | music/tracks/library_items, limit and offset |
| Search | music/search, track media type and library_only=true |
| Players | players/all |
| Active queue | player_queues/get_active_queue and player_queues/items |
| Play or add | player_queues/play_media with explicit play or add option |
| Controls | player_queues/play, pause, previous, next and stop |

Pages contain at most 50 tracks, the UI requests 25, and player lists contain at most 200 entries. Queue display is limited to its first 50 entries. HTTP responses use the shared 8 MiB bound and 15-second timeout. The supported operations are fixed in Rust. Caller-controlled API commands, raw media URLs and broad player targets are refused.

The initial implementation searches the current account's Music Assistant library. It does not perform cross-service catalog search, metadata export, provider enrollment, source copying, group creation, or stream forwarding.

## Upstream provenance

No Music Assistant, Home Assistant or proprietary Omnidia source was copied into this implementation. The adapter was independently written against inspected public API definitions.

Observed on November 26, 2026:

- Music Assistant server development revision 3f38952f01e670c1fa31874055dcff2ec4367a87, Apache-2.0: https://github.com/music-assistant/server/tree/3f38952f01e670c1fa31874055dcff2ec4367a87
- Music Assistant models revision c522d3e970588a4b1b8a06f5f2fb41260f8dd961: https://github.com/music-assistant/models/tree/c522d3e970588a4b1b8a06f5f2fb41260f8dd961
- Public API guide: https://www.music-assistant.io/api/
- Home Assistant Music Assistant integration: https://www.home-assistant.io/integrations/music_assistant/

Relevant inspected source includes webserver/controller.py, webserver/api_docs.py, players/base.py, player_queues/controller.py, music/controller.py and music/media/tracks.py under music_assistant/controllers, and the user and player models. The app interrogates the actual connected server because the development source and documentation can differ from an installed release.

## Provider rights

An authenticated Music Assistant command is not legal clearance for every downstream provider. Spotify's developer policy restricts integration with streams or content from other services and single-source simultaneous listening. Apple MusicKit's terms require its authorized player path and restrict download, redistribution and synchronization. The connector does not introduce a bypass or promise these uses are permitted.

- Spotify policy: https://developer.spotify.com/policy
- Apple Developer Program License Agreement, MusicKit: https://developer.apple.com/support/terms/apple-developer-program-license-agreement/

Source-specific rights and entitlements still apply to playback. Cross-account sharing, offline copying and watch-party redistribution require separate authorization and provider support.

## Qualification and follow-on work

This source change includes Rust tests for request routing, selected-player commands, response minimization, schema incompatibility, bounds, account/server mismatch and error handling. Execution results must be attached to the exact compiled source through the existing qualification workflow. Source tests alone do not establish a device build or a live Music Assistant journey.

Remaining product scope includes an actual account-to-search-to-speaker acceptance run, Apple builds and Keychain lifecycle checks, Music Assistant OAuth enrollment, live queue subscriptions, broader authorized media types, a Bluefin Sendspin player, and browser/other-platform adapters.

For synchronized audio, the candidate is Sendspin/sendspin-rs at 0db21ea08a341f21521fa28a6a88ba0c799df346, version 0.4.0, against Sendspin/spec at 2cfe19725e26087801985e1d47b31c437bb39099. The Rust repository remains labeled work in progress. Its declared MIT OR Apache-2.0 license, actual license files, protocol compatibility and physical timing must be checked before import. The current protocol's classical Noise channel does not establish Bluefin's post-quantum profile.

Independent peer-storage grants, global deduplication and cross-platform watch parties remain separate work. A storage grant must never authorize media search or playback, and one peer's invitation must never delegate another peer's resources without that owner's authority.
