# Native integrations

These files implement platform adapters, not a provider subscription service. Default builds require none of the optional entitlement templates. Minimum iOS/tvOS remains 15. App Intents and App Shortcuts compile only for iOS and require iOS 16 at runtime. No account enrollment, publishing, TestFlight, paid API call, upload or schedule is part of this change.

## Root integration hooks

1. Add `NativeIntegrations.attach(to: store)` after constructing the single app-owned LibraryStore, before presenting integration actions. Repeated attachment to that same store is harmless. Keep this attachment across view changes; call `NativeIntegrations.shared.detach()` only when retiring that store. Session observation clears Spotlight and leaves SharePlay on account changes. Do not detach just because the app backgrounds.
2. Present `NativeIntegrationActions(item: item).environmentObject(store)` on item detail pages. iOS gets a native title-only share sheet. No library URL, token, media URL, local server address or peer credential is shared. tvOS omits the unavailable iOS share sheet.
3. Set `NativeIntegrations.shared.openLibrary` to select the root library tab. On iOS 16+, call `BluefinAppShortcuts.updateAppShortcutParameters()` after attachment. Shortcut execution is foreground-only and requires the live app store. Play asks for a title, searches one bounded page and accepts only one exact playable match with a server-provided total proving no unexamined results. Pause/resume use the existing AVPlayer. This is App Intents, not the SiriKit media-domain extension or HomePod default music-provider support. iOS 15 has no new Siri shortcuts from these files.
4. Spotlight is opt-in per account. After the user chooses to expose titles in device search, call `try NativeIntegrations.shared.donateToSpotlight(items)`. It donates at most 100 non-container items, with a seven-day expiration. Route `.onContinueUserActivity(CSSearchableItemActionType)` to `continueSpotlight(_:)` in a Task and surface errors. Imports require CoreSpotlight on iOS. Account keys hash server plus user ID; identifiers include only that hash and an encoded item ID. Titles are deliberately visible to system search after consent. Queue ordering makes account deletion follow outstanding donation; deletion errors are not currently surfaced, so this is not a guaranteed erasure receipt. A mismatched account cannot activate an old result. Consent persistence/UI belongs to the root; never automatically donate after account switching.
5. With `BLUEFIN_SHAREPLAY` enabled, show `NativeSharePlayControls()` in a visible root/player location. The detail action offers SharePlay activation only after a user tap. Incoming sessions remain pending until the user taps Join. The adapter observes `$playback` and attaches `player.playbackCoordinator.coordinateWithSession(session)` before PlaybackController's asynchronous start. A retained AVPlayerPlaybackCoordinatorDelegate supplies server/item identity independently of participant-specific private stream URLs. No additional player hook is needed for this AVPlayer implementation. Leave pauses the local player and calls GroupSession.leave; it does not end the group for everyone. Account change, detach, invalidation and changed group content leave the session. Only one item per group session is supported; changing content requires a new user-activated session. No messenger, raw media or stream URLs are sent.

Parent must preserve an authenticated-session generation across each awaited LibraryStore.play request and recheck it before assigning playback, and honor cancellation before assignment. This store-owned protection is required for any asynchronous playback caller, including Siri, CarPlay and SharePlay. Integrations check identity around their own detail/search awaits but cannot make the parent's playback planner atomic. Root owns error display and the actual tab selection. Existing remote-command targets and Now Playing metadata remain owned by PlaybackController; these adapters do not register duplicate handlers.

## Provider linking, discovery and accounts

ExternalUrls must be server-confirmed links for this exact item, not search guesses. The adapter consumes Codable `ExternalUrls: [{"Url": "https://..."}]`; it intentionally ignores ProviderIds. Jellyfin/local item IDs, TMDb IDs and IMDb IDs are never substituted into provider-specific paths.

The bounded allowlist accepts explicit Spotify content URLs at `open.spotify.com`, Apple Music catalog links at `music.apple.com`, and Apple TV catalog links at `tv.apple.com`. URLs must use HTTPS, have no credentials, port or fragment, and match bounded content paths. Query parameters are rejected except Apple Music's numeric `i` song selector. Tracking queries must be removed by the metadata owner after checking the target, not blindly forwarded. Other providers, YouTube, redirectors, arbitrary websites and custom schemes are omitted until their exact link contract and content provenance are admitted. The allowlist validates syntax and origin, not catalog existence, subscription, territorial rights or stream entitlement. At most eight deduplicated links appear.

A user tap first opens with `universalLinksOnly`. Failure means the link did not open as a universal link, not that an app is absent. The user can then choose the website or the official App Store listing. iOS uses SKStoreProductViewController; a failed product load dismisses it. tvOS opens the HTTPS App Store listing and makes no claim that this iOS product is available on tvOS. There is no installation without user choice and no external playback started on launch. The share sheet shares only the title, never provider metadata wholesale.

No installed-app inventory is collected. `canOpenURL` cannot enumerate installed applications or prove a subscription; no LSApplicationQueriesSchemes entries are required here. Apple's current iOS 27 documentation caps those entries at 25 for apps linked on/after iOS 27.

No generic TV-provider SSO is implemented. [VideoSubscriberAccount](https://developer.apple.com/documentation/videosubscriberaccount) requires a participating provider relationship, the applicable entitlement and a real provider authorization flow. A TV-provider identity is not a licensed playback grant. Bluefin cannot read other apps' accounts, Apple ID credentials, cookies or subscriptions. Playback inside Bluefin still uses LibraryStore's independently authorized server planner and AVPlayer. Commercial provider playback needs an approved SDK/license, provider authentication and DRM entitlement where applicable. These files contain no extraction or bypass path.

## Optional provisioned capabilities

Keep all three flags OFF for the current free Personal Team build. Templates under Integrations/Templates are examples, not active build settings or profiles. Do not point CODE_SIGN_ENTITLEMENTS at them automatically. Root owns project.yml, scene setup and any future provisioned target. A simulator typecheck is not a signing or entitlement acceptance result.

- `BLUEFIN_SHAREPLAY`: merge the group-session entitlement only into a legitimately provisioned capability-enabled build. GroupActivities is gated out of the default app. Each participant installs Bluefin, signs in independently, and must have access to the same configured server and item ID. Only a SHA-256 server-identity digest and opaque server-local item ID are serialized. No account ID or token is shared. Server identity includes the configured credential-free URL, so aliases/trailing slashes must match. The digest is a namespace, not authentication or proof of server ownership. Two cloned servers with the same URL/IDs are outside this trust model. PlaybackCoordinator synchronizes the separately authorized streams; it does not distribute media. FaceTime/system eligibility and a provisioned multi-device run remain untested.
- `BLUEFIN_CARPLAY`: requires approved `com.apple.developer.carplay-audio`, appropriate paid membership/provisioning and an opt-in CPTemplateApplicationScene manifest. Merge the scene template into the existing manifest rather than replacing the phone scene configuration. It is unavailable on the user's current free account. The scene uses CPListTemplate, CPTabBarTemplate and CPNowPlayingTemplate. Only Type == Audio reaches playback. Recent audio and favorites come from the current home snapshot; favorites are the server's real IsFavorite flags, not a complete all-library favorites query or a local mutation. Music library browsing loads one page, omits containers and limits output to 60 tracks. A folder-only library page can therefore be empty. No recursive scan, pagination, search template or favorites mutation is implemented. Async row handlers defer completion; tasks cancel on disconnect/reload and check generation. No custom Apple Music screen, video template or entitlement approval is claimed. Phone store must attach before a CarPlay connection; a connection before attachment shows the sign-in/open-app prompt and requires reconnecting.
- `BLUEFIN_HOMEKIT`: iOS-only, requires HomeKit entitlement/profile and an honest NSHomeKitUsageDescription in the root plist. Create NativeHomeScenes only from a Home settings action and invoke requestHomes after explicit user choice. Present the returned homes and their actual actionSets, let the user select a scene, then call execute(_:in:) only from an explicit control. There is no launch-time home permission request, autonomous trigger, accessory invention or media-player registration. HomeKit scenes may control real accessories; root must label that action accurately. Permissions, denied access, home availability and execution must be exercised on an authorized device. No HomeKit UI is injected by this adapter.

## Home Assistant and music transport boundaries

No Home Assistant credential UI or network client is implemented. A correct direct client still needs explicit endpoint setup, HTTPS validation, user-entered token in a device-only Keychain item, token revocation/removal, bounded read-only state requests, user-selected service controls, request cancellation and per-account separation. No token belongs in a source file, shared URL or Siri phrase.

The supported manual integration path is the user's own Apple Shortcuts workflow, following the [Home Assistant Companion documentation](https://companion.home-assistant.io/docs/integrations/siri-shortcuts/): combine the Bluefin App Shortcut actions with Home Assistant Companion's user-configured actions or a user-owned Home Assistant webhook. In current Companion versions, Perform action needs iOS 17 or later. Select the already configured server, choose the intended action and supply only the explicit entity data. For read-only composition, choose an appropriate read action instead. A user-owned webhook follows the [Home Assistant webhook trigger contract](https://www.home-assistant.io/docs/automation/trigger/#webhook-trigger). Keep webhook URLs inside that user's shortcut because the webhook ID is a credential. Do not donate/share those URLs. Do not add an unattended schedule. Home Assistant webhook automation setup is a separate explicit server task; it is not configured here. This path does not turn Bluefin into a HomeKit accessory, HomePod music service or arbitrary Home Assistant player.

Lossless/direct-Iroh transport, stream authorization, codec/sample-rate negotiation, favorites mutation and matched iOS/tvOS music UI belong to the parent/Rust owner. These adapters call LibraryStore only, with no new C FFI entry points or raw peer secrets. The existing chain is LibraryStore -> RustBridge -> existing C ABI in include/bluefin.h -> Rust planner/transport, then PlaybackController -> AVPlayer. This Swift/platform work is not an all-Rust implementation or proof of lossless output. CarPlay uses the exact same authorized player and remote commands.

## Parent integration additions

The root app now attaches the store, selects its actual Library tab for the
open-library shortcut, routes Spotlight activities, and presents native actions
on item details and the dedicated music player. Account generation guards
protect asynchronous library and playback requests.

CarPlay source now includes video browsing for sessions reporting
`supportsVideoPlayback`, with `CPPlaybackConfiguration` preferring video for
eligible items. Apple controls temporary video availability and audio-only
fallback. The inactive entitlement template includes audio and video, both
requiring legitimate provisioning and approval before use. This does not claim
compatibility with a particular vehicle, AirPlay receiver, or the private
loopback stream bridge. CarPlay video still needs an end-to-end device check.

Home Assistant now has a separate native connection screen and Rust REST
adapter. It stores its own token in Keychain, reads a bounded projection of
media-player state, and sends only explicit play/pause, next, and stop actions
to one selected entity. It never forwards the media-server token or private
stream URL. No household Home Assistant server was configured or controlled
during development. OAuth account setup, media-player entity registration,
stream casting, event-driven state sync, and automatic home scenes remain
unimplemented. The manual Shortcuts composition described above remains useful.

## Verification and primary references

The worker ran SDK typechecks for iOS 15 and tvOS 15 deployment targets against the installed Xcode 27 SDK, including optional capability sources. Private logs and exact hashes are in the worker handoff. No simulator/device control, StoreKit product load, Siri recognition, Spotlight launch, SharePlay group, CarPlay connection, Home permission prompt, provider playback or signed installation was executed. The parent's driver failed before this assignment and tvOS has no listed device. TestFlight remains blocked on paid enrollment and no public publishing is requested.

References checked on 2026-11-08:

- [Universal links](https://developer.apple.com/documentation/xcode/supporting-universal-links-in-your-app), [canOpenURL limits](https://developer.apple.com/documentation/uikit/uiapplication/canopenurl(_:)).
- [Spotify content linking](https://developer.spotify.com/documentation/ios/tutorials/content-linking), [Apple Music catalog URL examples](https://developer.apple.com/documentation/applemusicapi/get-a-catalog-playlist), [Apple TV catalog](https://tv.apple.com/).
- Official product listings: [Spotify](https://apps.apple.com/app/id324684580), [Apple Music](https://apps.apple.com/app/id1108187390), [Apple TV](https://apps.apple.com/app/id1174078549).
- [App Intents](https://developer.apple.com/documentation/appintents), [App shortcuts](https://developer.apple.com/documentation/appintents/getting-started-with-the-app-intents-framework).
- [Coordinated playback](https://developer.apple.com/documentation/avfoundation/supporting-coordinated-media-playback), [GroupSession](https://developer.apple.com/documentation/groupactivities/groupsession).
- [CarPlay](https://developer.apple.com/documentation/carplay), [audio entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.carplay-audio), [Home scene execution](https://developer.apple.com/documentation/homekit/hmhome/executeactionset(_:completionhandler:)).
