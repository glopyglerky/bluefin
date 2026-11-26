# Lyrics path and timing compatibility

Reviewed on 2026-11-26 against the repository's current source pin:
**Jellyfin v12.4, commit `83d0d0e7f7`**
(`handlers::UPSTREAM_TAG` / `handlers::UPSTREAM_COMMIT`).

## Stored lyric paths

Imported media-stream rows can store `%MetadataPath%` or `%AppDataPath%`
instead of an absolute filename. The catalog reports `HasLyrics` from the
presence of those rows. Reading the stored token as a literal filesystem path
then returns no lyrics even when the corresponding file exists.

The pinned upstream path is:

| Component | Compared behavior |
| --- | --- |
| [MediaStreamRepository](https://github.com/jellyfin/jellyfin/blob/83d0d0e7f7/Jellyfin.Server.Implementations/Item/MediaStreamRepository.cs) | `GetMediaStreams` maps each stored row. `Map(MediaStreamInfo)` assigns `dto.Path = RestorePath(entity.Path)`; `RestorePath` calls the application host's path expander. |
| [ApplicationHost](https://github.com/jellyfin/jellyfin/blob/83d0d0e7f7/Emby.Server.Implementations/ApplicationHost.cs) | `ExpandVirtualPath` replaces the data and metadata tokens using the current application paths and ordinal case-insensitive matching. |
| [LyricManager](https://github.com/jellyfin/jellyfin/blob/83d0d0e7f7/MediaBrowser.Providers/Lyric/LyricManager.cs) | `GetLyricsAsync` reads each lyric stream's resolved path and tries the registered parsers. `DeleteLyricsAsync` deletes the resolved stream files through the existing library-monitor lifecycle. |

Bluefin's media-stream repository returns stored entity rows, so the shared
lyric manager must perform that expansion before file access. It now receives
the existing `VirtualPathExpander` from server composition and uses it in
`lyric_files`. Both reads and the existing authorized delete operation follow
the current server directories. Absolute paths retain their values.

The expander shares the server's live path state. A metadata-directory change
takes effect for subsequent lyric reads and deletes without reconstructing the
manager.

## Timestamp range

The pinned
[LrcLyricParser](https://github.com/jellyfin/jellyfin/blob/83d0d0e7f7/MediaBrowser.Providers/Lyric/LrcLyricParser.cs)
converts line and cue milliseconds through `TimeSpan.FromMilliseconds(...).Ticks`.
Its cue positions index .NET strings, so the wire positions are UTF-16 code
units; cue times are absolute song times.

The Rust parser previously bounded millisecond arithmetic but subsequently
multiplied accepted values by 10,000 without checking the signed 64-bit tick
range. An oversized source tag could panic in a debug build or wrap in a
release build. The parser now checks that conversion before accepting the tag.

Representable LRC/ELRC times retain their existing values. An out-of-range tag
follows the existing invalid-tag behavior: it stays literal when embedded in a
valid timed line, and a document without valid timed lines can fall through to
the plain-text parser. This malformed-input handling is an explicit safety
choice; identical exception behavior to upstream outside the tick range is
not claimed.

## Contract and parity-record scope

The HTTP routes, PascalCase lyric DTO, 100-nanosecond tick units, UTF-16 cue
indices, parser ordering, item visibility, and lyric-management authorization
are unchanged. This fix restores the shared path-resolution behavior behind
the existing GET and DELETE operations. It adds no client-supplied path,
provider lookup, schema migration, or content acquisition.

This record covers path expansion and timestamp conversion at the current
upstream pin. The GET/DELETE lyric operation rows in
`handlers::VERIFIED_AGAINST_EARLIER_PIN` retain their existing earlier-pin
status because the separately recorded `gl-408` controller-binding gaps remain
open. This scoped comparison does not re-date those complete operation rows.

## Validation

The exact server source lineage passed all **47** `lyric_manager::tests::`
tests with the pinned Rust 1.99.1 toolchain, offline locked dependencies, two
build jobs, and development/test debug information disabled:

```sh
cargo test --offline --locked -j 2 -p ferrofin-core --lib lyric_manager::tests:: -- --test-threads=2
```

The new `adopted_lyric_paths_follow_live_server_directories` regression uses a
real test database and temporary files. It covers mixed-case metadata tokens,
the app-data token, preserved word-cue timestamps, a changed metadata root,
and deletion of only the current resolved file.

The new `lrc_out_of_range_timestamps_do_not_corrupt_playback_timing` regression
covers overflowing line and word tags and the largest whole-millisecond value
that fits the wire tick range. The modified Rust file also passes the pinned
formatter check.

**Upgrade note:** none. Existing lyric stream rows and files are read in place.
