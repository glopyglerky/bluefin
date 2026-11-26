# Bluefin appearance, Home, and search

## Product requirements

Use the approved mist, deep-navy, and muted-lavender appearance in both system
appearances. Retain the Bluefin fin and waves, with a static fine texture behind
content. Media art keeps its own colors. Every client uses Bluefin product
branding.

The primary categories, in order, are Home, Sports, TV Shows, Movies, Live TV,
and Music. Search, account controls, and collections remain accessible utilities.
Phone layouts must expose all six categories; TV controls must remain reachable
with the remote.

Home belongs to the signed-in account and its owning server. Continue watching,
next episodes, favorites/likes, and explicit submitted searches may inform its
shelves. Do not record typeahead text. Cold-start shelves must describe actual
library media, without pretending to know a new user's preferences.

Search defaults to all media and can narrow to media type, studio, a server tag,
or a collection. A server may use tags for streaming services or custom groups.
Every page must carry the same filters; filtering only the first downloaded page
does not satisfy this requirement. Only metadata and media the account may access
are included. Independent server accounts are not silently merged.

Use the existing authenticated HTTPS or paired device route for account data.
Keep QR pairing, peer identity checks, playback, and account cleanup intact.
Search history is stored by the account's server, bounded to 64 events and 90 days,
and clearable by the account. A delayed request cannot repopulate cleared history.

Artwork must follow advertised item IDs and image versions. Episode and season
poster cards prefer their series poster; songs can inherit their album cover.
Reuse concurrent image work, downsample to display sizes, bound caches and prefetch,
and reserve request capacity for navigation and playback. Missing provider artwork
uses an honest themed placeholder. Network playback and cold remote loads require
measurement; zero-latency claims are not an acceptance criterion.

## Current server transport

Managed QR pairing establishes an authenticated session on the account's own
self-hosted server. Home recommendations and search history use that server's
authenticated API. The existing Iroh media transport is retained. The Apple
client can use the newer paired account-control protocol, but that backend
protocol is not part of this production Home release. Independent server
accounts remain separate; pairing does not merge or replicate their histories.
Transport tests must identify whether they exercised the installed route,
an isolated loopback media server, or only a pinned-peer handshake.

## Release qualification

Compare actual iPhone and Apple TV screenshots against the approved appearance.
Build from a reviewed snapshot with the existing signing identity and bundle ID.
Upgrade existing installations so accounts and app data survive. Retain exact
installed rollback bundles and verify the newly installed version and launch.

The production server must be based on its exact running source, retaining its
current guide, catalog, recording, and authorization behavior. Apply only the
focused additive Home/history changes to that base. Qualify the real authenticated
HTTP routes and account isolation before switching the service. Preserve its
state directory and existing deployment configuration.

Other client source changes require their applicable compiler and behavior checks.
Source-level checks alone do not establish a public store release or acceptance
on an unavailable physical device.
