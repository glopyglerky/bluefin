# Public source and service boundaries

Bluefin is a public, reusable media server and client project. A user's media,
accounts, library database, watch history, private service addresses, server
configuration, signing identities, keys and operational evidence are runtime
data. None belongs in its source, fixtures, screenshots, app bundle or releases.
Examples use reserved example domains and synthetic accounts. Release review
must inspect the exact source generation and built bundle, including resources
and generated configuration. Keeping a private file untracked is not sufficient
if a build copies it into the bundle.

The included preview video and audio are original generated Bluefin samples.
Preview playback uses bundled files and does not contact a media provider.
`tools/GeneratePreview.swift` and `Tests/NativePlaybackFixtures.swift` reproduce
them with Apple's native encoders. No personal library is a demo fixture.

## Separate projects

Bluefin owns media catalog/browsing, authenticated serving, native playback,
per-user playback state, permitted offline copies, and its transport contracts.
The arr stack remains external: Sonarr, Radarr, Lidarr, Readarr, Prowlarr, Bazarr
and similar projects are not Bluefin components. Do not vendor, reimplement,
bundle, install, start, upgrade or orchestrate them in this repository or app.
Do not make them prerequisites for running Bluefin.

Optional integrations may call a separately installed, user-configured service
through its supported API with that service's own authorization. Configuration
and credentials stay outside source. A generic authenticated library-change
notification or NFO reader is compatibility with external producers, not an
embedded arr application. Preserve those interoperability contracts. Pilfrr,
Lustr and BlueFe also retain their own repositories and responsibilities.

## Upstream work and provider access

Keep Ferrofin/Jellyfin attribution, the repository GPL-3.0 license, dependency
licenses, and vendored notices. Match useful behavior and native design
principles; do not copy proprietary app source, artwork, branding or screenshots.
Code adapted from another project requires a recorded origin and compatible
license. Publish the corresponding source required by its applicable licenses
when distributing binaries. The license and distribution terms for each release
channel, including the App Store, require review before that channel is used.

Provider integrations use documented APIs or explicit validated content links
with user-initiated native handoff. An installed app, a TV-provider selection,
an external identifier or a search result does not establish an entitlement to
play content. Bluefin does not scrape provider credentials, extract protected
streams, strip DRM, bypass subscriptions, spoof an official client or download
provider media without authorization. Playback inside Bluefin requires an
authorized supported integration. Otherwise open the provider's own app or site.
Server download permission is an application permission, not proof of copyright
ownership or a provider's redistribution license.

Apple's [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/)
sections 5.1 and 5.2 address privacy, third-party permissions and media downloads.
Each provider's own terms still apply. Unsupported or unapproved capabilities
remain disabled; do not invent entitlement or partner access. No public release,
App Store upload, enrollment, payment or account-name change is authorized by
this development work.

## Runtime privacy

The app stores its selected viewer session in the local Keychain. Library queue,
download and artwork data are scoped to the selected account. Sign-out drains
playback cleanup and clears session access. Share, Spotlight donation, SharePlay
and home integrations require deliberate user actions; credentials and playback
capability URLs must never be shared or logged. A participant uses their own
authorized server session. A hash is a correlatable identifier, not a promise of
anonymity. The privacy manifest and store disclosures must match the final
enabled integrations and exact dependency graph.

Release evidence stays private and records source/artifact hashes, dependency
and asset licenses, secret/personal-data checks, enabled capabilities and actual
device/server acceptance. A source review or successful build alone cannot
certify universal legal compliance or whole-system acceptance.
