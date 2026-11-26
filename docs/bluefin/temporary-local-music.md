# Temporary local music in the browser

local-music.js is a small platform adapter for explicit browser File selections. It works without Wasm, IndexedDB, service workers, WebCrypto digest, accounts, or a remote server. It uses crypto.getRandomValues only for opaque selection IDs.

The selected File objects remain in a private page-local map. Selection reads at most 64 bytes per file to reject mismatched containers and playlists. It does not buffer or hash full audio, copy it into persistent storage, crawl a directory, request a provider entitlement, upload metadata/bytes, or add files to anyone else's search. Playback reads the selected File on demand through one object URL.

## Identity and ownership

Records carry source user_file_import and identity opaque_file_selection. IDs have the form local-file- followed by 64 random hexadecimal characters. They are deliberately incompatible with the durable Rust catalog's SHA-256 item IDs. Selecting the same name or bytes twice creates two selections; no deduplication or content-equivalence claim is made.

Possessing a serialized record or ID does not grant access: acquireSource and queue operations require a live File in the same controller's private map. No restore/import-record method exists. Source provenance records the caller's explicit file-picker selection; it is not a copyright or redistribution license and is not a peer grant.

The native Rust model remains the owner of durable device catalogs. This adapter preserves its local-import distinction, queue limits, cursor/deletion behavior, and repeat semantics, while keeping the browser's temporary File selections a separate identity type. It does not try to impersonate a durable Rust catalog in a browser where Rust/Wasm cannot run.

## Limits

- One active local-library controller per page.
- One physical metadata read at a time across controllers; clearing/closing does not prematurely release this admission.
- At most 20 Files per selection, 512 retained selections, 2 GiB per File, and 16 GiB total declared File sizes.
- At most 512 queue entries and one active playback object URL.
- Titles are bounded to 512 UTF-8 bytes; local search is bounded to 256 UTF-8 bytes.
- No whole-file hashing or small audio-memory cap is required to play an ordinary lossless file.

The total is declared referenced bytes, not a claim that 16 GiB has been copied or allocated in browser memory. Actual media decoder memory and operating-system buffering are separate platform behavior. The application never starts multiple local media decoders through this adapter.

Container sniffing admits MP3, ADTS AAC, M4A, FLAC, WAV, and AIFF/AIFC. It does not establish decoder support, DRM status, or legal rights. The consuming HTML media player must handle unsupported/error/encrypted media without requesting keys or creating a service download path.

## Integration contract

Load local-music.js as a normal same-origin script before the progressive native view. It defines the frozen global BluefinLocalMusic with version 1, limits, isLocalId(id), and create({onChange}).

The returned controller exposes:

| Method | Behavior |
| --- | --- |
| snapshot() | Immutable bounded metadata; never includes a File or a persisted secret |
| addFiles(arrayOfFiles) | Checks one bounded selection and publishes it together; returns added opaque IDs and the current catalog |
| search(text) | Filters only this controller's private local metadata; no network request |
| setQueue({items,currentIndex,repeatMode,shuffled}) | Accepts only IDs backed by current local File objects |
| select(id) | Selects an existing queue occurrence or a one-item queue |
| advance(forward,automatic) | Deterministic cursor/repeat transition; returns the next local ID or null |
| acquireSource(id) | Returns a single URL lease with id, url, mimeType, signal, release(), metadata(durationSeconds) |
| remove(id) | Removes only that selection and its queue entries, aborting/releasing its active URL |
| clear() | Drops all File references and queue metadata and invalidates in-flight selection publication |
| dispose() | Permanently retires this controller and releases its sources/references |
| subscribe(callback) | Adds a bounded metadata observer; returns an unsubscribe function |

All calls that accept an ID require a live privately held File; isLocalId is only a routing hint, never authorization.

The progressive UI owns the one HTML media element. Before local playback it must stop/release the preceding server/mesh/local player. It then obtains a lease and assigns its blob URL to that element. The lease's abort signal must stop the element and remove its src. Revoking a URL alone does not invalidate a decoder that already opened it. Stop the player before calling release(). Use the lease identity/generation when handling events so a late event cannot stop a successor.

On loadedmetadata, pass a finite positive duration to lease.metadata(duration). This optional duration is presentation metadata and grants nothing. Unsupported media should produce an explicit playback message and release its lease. Keep private local items out of server search, remote queue synchronization, server playstate reporting, and invitation resource selection.

The module synchronously disposes on pagehide and refuses to create another owner in a retired page. A new full page load requires the user to select Files again. It makes no claim that asynchronous remote cleanup can finish during browser shutdown; this module has no remote session.

## Qualification

Eight fixture tests in tests/local-music.test.cjs cover:

- a no-Wasm/no-storage environment with large declared lossless files and only bounded prefix reads;
- equal-name/equal-byte selections remaining distinct, with forged/serialized IDs unable to access Files;
- one active URL and stale-release isolation;
- page retirement, reference cleanup, and late metadata refusal;
- clearing during a physical metadata read without resurrection or premature admission release;
- batch, size, byte-accounting, and File-provenance limits;
- queue deletion/repeat and local-only search;
- invalid-container atomicity and reported URL-cleanup failure.

Run these only through the admitted Forge JavaScript executor. They exercise the File/platform boundary using controlled fixtures; they do not qualify Safari Lockdown Mode, a real audio decoder, a physical iPhone, or the complete integrated UI. The browser owner is responsible for wiring this isolated module into the native-style player and shipping bundle.
