# Native trailers

Bluefin browses trailers associated with titles in the authenticated server library. Local trailer extras use the existing native playback planner and controller. Remote trailers open their provider's public page or installed app. They are never resolved into stream URLs.

## Apple integration finding

Apple documents trailer selection on title pages in the [Apple TV app](https://support.apple.com/en-asia/guide/tvapp/atvb60881377/web). No supported standalone Apple Trailers app integration or public unauthenticated Trailers catalog API was established. The [Apple TV Feed](https://developer.apple.com/documentation/appletvfeed/) is a separate metadata export API. Its [token documentation](https://developer.apple.com/documentation/appletvfeed/generating-developer-tokens) requires credentials supplied through the Performance Partner Portal. Bluefin does not assume access or ship those credentials. A future authorized server metadata importer can provide exact Apple TV title links.

Native handoff uses Apple's [universalLinksOnly](https://developer.apple.com/documentation/uikit/uiapplication/openexternalurloptionskey/universallinksonly) option. Successful opening does not establish account access, playback rights or trailer availability. iOS offers an explicit official website fallback. tvOS explains when no installed app accepts a link, since it cannot assume a web browser.

## Parent integration

1. Add `mod trailers;` to the Rust bridge root. Dispatch `"trailersCatalog" | "trailersDetail" => trailers::call(&api, operation)` from its authenticated operation match. Use the existing match binding's operation variable name. No other bridge changes are needed.
2. The existing XcodeGen source directories include the new Swift and test files. Regenerate the project through the parent workflow.
3. Add `NavigationLink(destination: TrailersView())` inside the shared navigation hierarchy and pass its existing LibraryStore environment. For a title detail action use `TrailersDetailView(itemID: item.Id)`. The view does not require a new global store or a separate player.
4. Keep the parent's existing playback presentation. Trailer selection calls `LibraryStore.play` with the actual local extra, starting from the beginning. That planner retains native format admission, authenticated transport, progress and cleanup behavior. The owner movie is never substituted.

## Transport contract

Both operations use the existing bridge session fields and same-origin Api client, Authorization header, bounded response reader and redirect refusal. They preserve an optional server base path. Their routes follow the public [Jellyfin ItemsController](https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/ItemsController.cs) and [UserLibraryController](https://github.com/jellyfin/jellyfin/blob/master/Jellyfin.Api/Controllers/UserLibraryController.cs), also implemented by Bluefin's server.

- `trailersCatalog`: optional `startIndex` from 0 through 1000000 and `limit` from 0 through 60, default 30. GET `Items` with UserId, Recursive=true, IncludeItemTypes=Movie,Series, HasTrailer=true, Fields=Overview,RemoteTrailers,LocalTrailerCount, SortBy=SortName, StartIndex and Limit. Returns standard `Items` and optional `TotalRecordCount`. The native screen pages 30 titles at a time.
- `trailersDetail`: required exact `itemId`. GET `Items/{itemId}` with UserId and detail fields, then GET `Items/{itemId}/LocalTrailers` with UserId. Returns `{item: BaseItemDto, localTrailers: [BaseItemDto]}`. Detail must retain RemoteTrailers; do not run it through an unrelated field projection. A denied or failed extras request is an error, not an empty list. The Rust module rejects missing, duplicated and owner-equal extra IDs.

The detail screen reissues `trailersDetail` before local playback and requires unique membership under the exact requested owner. A changed account, credential, item or cancelled view task cannot publish its old response or begin playback. SwiftUI view tasks own request lifetime. Synchronous Rust HTTP work can finish after cancellation, but its result is discarded; existing transport timeouts bound that work.

RemoteTrailers are bound to the returned owner ID. Only HTTPS exact-host YouTube watch/short links, Vimeo numeric video pages and Apple TV movie/show detail links are shown. URL credentials, ports, fragments, escaped paths, unknown hosts and extra query parameters are rejected. Provider names come from the allowlist. Unknown links are omitted. There is no URL extraction, embedded third-party player, provider login capture or provider entitlement claim. Server authentication and parental/media access remain authoritative.

## Qualification boundaries

The trailer tests use original synthetic JSON and loopback HTTP responses, with no downloaded trailer assets. XCTest covers exact owner/extra association, duplicate rejection and provider URL attacks. Rust tests cover scoped authenticated requests, paging, owner/extras routes and forbidden responses. Compilation and execution results belong in the private integration handoff. Source-only hooks do not establish a wired app or installed playback acceptance. Parent qualification must exercise loading, retry, empty pages, account switch during loading/preparation, local trailer playback and cleanup, provider app opening and unavailable-provider messaging on iPhone, iPad and Apple TV. No device session is owned by this lane.
