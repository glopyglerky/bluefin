# Watch companion and Siri

Bluefin Watch is an independent companion of `org.glopyglerky.bluefin`.
The iPhone app embeds the Watch app. The Watch bundle declares
`WKCompanionAppBundleIdentifier` and `WKRunsIndependentlyOfCompanionApp`,
with no `WKWatchOnly` key. Device builds retain arm64 and arm64_32; the
qualification simulator is arm64. Signing and distribution remain separate.

## Paired setup

The Watch asks its paired iPhone for an HTTPS server, viewer ID and a nonsecret
account generation through an interactive WatchConnectivity message. The phone
binds a single-use, 60-second nonce grant to its saved account generation and
active pairing. Paired setup requires the server to explicitly advertise
`managed_pairing_v1`; it does not silently fall back to legacy Quick Connect.
The Watch initiates a [managed challenge](managed-pairing.md) directly on that
server with its own stable device ID and independent client identity. It saves
the proof in device-only Keychain before asking the phone to authorize it.
The phone authorizes as its current viewer, without accepting a caller-selected
viewer override. The Watch exchanges the proof for its own session, then
rechecks the phone's server, viewer and generation before publishing it.
No phone password or phone access token crosses WatchConnectivity. Proofs travel
only in interactive messages and the documented HTTPS managed pairing API,
never application context or background userInfo transfers.

Interactive replies have a 12-second bound and settle their continuation once.
Network requests refuse redirects, enforce HTTPS and bound response size and
deadlines. Unsupported servers, failed cleanup and unreachable phone states
remain visible. Independent username/password sign-in remains available.
Bluefin does not enable server settings. The phone can read its saved session
without an attached foreground library view. Locked Keychain access, failed
storage, system wake policy or reachability can still prevent setup. Open
Bluefin on the iPhone when background authorization is unavailable.

Application context contains only a generation. A received generation change
fences paired playback and starts the existing cleanup/sign-out path. An offline
Watch cannot learn of a phone account change immediately. Before authorizing,
the phone saves a separate proof owner in its device-only Keychain. Phone
sign-out or account replacement cancels these owners before changing saved
credentials, even when the Watch is unreachable. Restart and activation retry
obsolete owners. Network or storage failure retains ownership and blocks
replacement; there is no promise of immediate revocation while offline.
Unpairing itself is not a cancellation receipt.

The Watch keeps the managed proof after publishing its session. Sign-out first
pauses local playback and drains playback leases, then cancels the proof.
Only a successful explicit `state=cancelled` receipt permits deleting that
owner. A timeout, 401, 404, expired challenge or failed Keychain operation does
not prove cleanup. Lost replies and interrupted setup remain retryable without
forgetting a session that may already have been minted. The phone's ledger is
bounded to 32 unresolved owners and refuses growth rather than evicting them.

An unresolved legacy Quick Connect challenge from an earlier app version keeps
its conservative exchange/logout recovery path. An expired or missing legacy
lookup does not prove revocation, so that recovery can still require server
session management. New paired setup uses the managed contract.

Once a token is received, failed setup cleanup retains its captured account in
a separate device-only Keychain slot and blocks replacement setup. Retry cleanup
before connecting again. Playback lease owners retain their account and release
path until successful cleanup. Stop, replacement and sign-out keep the existing
joined retry behavior and item-generation fences. Failed credential deletion
retains a retry state.

## Music

Watch playback runs on Watch using AVPlayer, long-form audio activation, native
Now Playing and remote transport commands. Music and favorites use server
pagination and search. The server queue preserves order and currently displays
up to 50 entries. Next/previous operate within the displayed collection; queue
edits are not implicitly persisted. Favorites write through the authenticated
server. Artist metadata appears when supplied. Seek uses the native remote
position command. Actual output routes, uninterrupted background audio and
physical-device accessibility still need hardware qualification. There is no
claim of offline downloads, lossless wireless playback, video or Sing.

The interaction references are Apple's [Watch music guide](https://support.apple.com/guide/watch/apd70768b20b/watchos)
and Pocket Casts' [Watch play source](https://support.pocketcasts.com/knowledge-base/apple-watch-watch-play-source/).
They informed clear playback location, Now Playing, queue and search behavior.
No proprietary implementation, artwork or icons were copied.

## Siri and shortcuts

On iPhone, library search and item information read the current authorized
server through the existing Rust client. Entity identifiers are bound to the
server, viewer and current session. Queries authenticate locally before exposing
personal titles. Intents require local device authentication, recheck account
identity after waits and do not donate library suggestions automatically.
Search returns at most 60 results and distinguishes a reported total from an
unknown total. Item information reports server metadata and library visibility;
it does not guarantee that a codec, route or server playback plan will succeed.
Native entity selection supports title disambiguation. The older title shortcut
requires one exact match within a complete search page and refuses fuzzy or
ambiguous matches. Foreground playback requires a real initialized LibraryStore;
it fails visibly if unavailable. Opening the app alone does not select a tab.

Watch has local search, exact-title playback and pause/resume, next/previous,
Stop and favorite intents. They operate on the authorized Watch account and
require local authentication. They do not remotely control the phone. Siri
matching, voice routing and personal-data presentation require system runtime
and hardware qualification. These intents support defined library searches and
item questions, not arbitrary conversation, recommendations or general Q&A.
Discovery requests use only administrator-enabled generic Bluefin actions.
A native action query returns the server-advertised choices. Submission requires
authorized-use confirmation and the server form notice in a native confirmation
dialog. Forms requiring option selection remain in the existing application UI.
Capability absence, permission denial and disabled support are unavailable;
clients do not enable them. Submission success is a server receipt, not proof
that the requested title is playable.
No native client may contain upstream connector settings or credentials.

Primary API references are [WatchConnectivity transfers](https://developer.apple.com/documentation/watchconnectivity/transferring-data-with-watch-connectivity),
[independent Watch apps](https://developer.apple.com/documentation/watchos-apps/creating-independent-watchos-apps),
[companion bundle metadata](https://developer.apple.com/documentation/bundleresources/information-property-list/wkcompanionappbundleidentifier)
and [intent authentication policy](https://developer.apple.com/documentation/appintents/intentauthenticationpolicy).
Build, native test runner, simulator startup, paired transport, audio-route and
spoken Siri evidence are separate results; see the implementation handoff.
