# Device-owned music in the Apple client

## Implemented source path

The welcome screen, Music browse grid, and Settings expose **Music on this device** without requiring a Bluefin server account. The native Files picker selects at most 20 files per action. Its security scope remains open through a coordinated, synchronous Rust copy. Rust records the content hash, title, format, size, import time, and local queue. The Apple adapter checks the copied file with AVFoundation and uses the existing PlaybackController with no remote account, bearer, playback reporter, or network plan.

Supported file containers in this initial profile are MP3, ADTS AAC, M4A, FLAC, WAV, and AIFF/AIFC. Actual codec playback depends on the Apple device. Protected content and video tracks fail the native suitability check. A failed check removes only a newly created imported copy; a preexisting identical item and the selected original are preserved. Every playback request repeats the content hash and native suitability checks.

The local player provides play/pause, seek, previous/next, repeat, a persisted queue, Now Playing/system controls, and the existing iOS output route picker. Server and local playback pass through one LibraryStore replacement gate. Starting server playback stops the local player; sign-out stops playback but never removes imported music.

## Storage ownership and privacy

The local index and media live in the app's Documents/BluefinMusic directory. The folder is included in normal device backups and uses complete-until-first-user-authentication file protection. The files are separate from the server-account offline cache, which is a disposable download cache.

The shared Rust model lives in crates/bluefin-contract/src/owned_library.rs. It has no account or network dependency. A random public library ID identifies the catalog; it is not a secret or an invitation. Item IDs are SHA-256 over the complete imported bytes. Matching hashes avoid duplicate copies within this local catalog, and confer no permission to discover, play, or redistribute another installation's files.

The source field accepts only user_file_import. It records the user's Files selection, not proof of copyright or redistribution rights. Peer caches, service streams, inherited access, and resharing flags are not accepted catalog variants. There is no API in this feature that sends its catalog to another person, adds its files to remote search, turns a service stream into an import, grants a peer access, or uploads files to a server or model. Normal operating-system backups remain under the person's device/account settings.

This is one private installation's local library. It is not a multi-person authorization system for people sharing an unlocked device. Future peer access must use a separately validated owner/grantee/content grant and may not treat catalog provenance or hash equality as authorization.

## Durable Rust operations

The native adapter is apps/bluefin-apple/rust/src/owned_music.rs and forbids unsafe code. It uses the existing descriptor-based local-storage wrapper for destination mutations. The selected source is opened as a regular, non-link file using the platform's safe OpenOptions wrapper; this does not require permission to list the provider's directory.

Import writes a retained operation record before creating its unique partial file. It copies through a 64 KiB buffer, checks source identity/size/timestamps, hashes all bytes, synchronizes the file and directory, and records the completed hash before publishing the final catalog entry. Restart reconciliation finishes a known complete import, removes a recorded incomplete partial, or preserves the prior catalog if unacknowledged import bytes are absent. It does not scan or delete unrelated directory entries.

Removal first records the exact retired item and removes its catalog/queue reference. It then deletes only that content file and clears the retained removal. An interrupted removal can be retried. Mutations require the catalog's library ID and expected revision; stale writes fail without a silent merge.

| Bound | Current source value |
| --- | --- |
| Single audio file | 2 GiB |
| Imported items | 10,000 |
| Catalog bytes | 8 MiB |
| Queue entries | 512 |
| Title | 512 UTF-8 bytes, no control characters |
| Files per picker action | 20 |
| Rust copy buffer | 64 KiB |
| Native preflight free-space reserve | 512 MiB |
| Revisions/timestamps | JSON-safe integers |

The free-space preflight is an estimate, not a reservation. Disk-write errors retain a recoverable operation. These explicit user imports do not enable background storage donation or an always-running peer service. Import work may be suspended or terminated by the operating system; the journal handles the next launch.

## Platform boundary and remaining work

The native screen uses existing Bluefin semantic colors, music rows, controls, and responsive SwiftUI sizing. It is available in the phone/iPad/Catalyst/Vision application source. Apple TV does not expose this import path, because its disposable local storage must not be represented as the only durable owner of a user's music.

This source does **not** embed a complete server into the phone, serve owned audio to Apple TV over Iroh, add new direct sharing grants, synchronize local playback positions between devices, or make the phone an always-running daemon. The existing NativeServer adapter remains queue synchronization against a remote server. Phone-to-TV direct serving is a separate unfinished resource/authorization journey.

## Qualification

The shared contract includes five tests for stable imports, conflicting metadata, stale revisions, queue deletion/repeat behavior, source/grant rejection, and container mismatch. The native adapter includes four tests for account-free dispatch and restart, duplicate import, symlink and stale-write refusal, crash reconciliation, and changed media rejection while preserving unrelated files.

The Forge owner was given these source checkpoints and the targeted gates:
- cargo test -p bluefin-contract owned_library
- cargo test --manifest-path apps/bluefin-apple/rust/Cargo.toml --features portable-apple-check owned_music

Those commands describe gates to be executed through an admitted Forge lane, not permission to run raw builds. At this checkpoint, the new Swift screen and its Files/player lifecycles have not been compiled or tested on a device. Project metadata parses and includes DeviceMusic.swift in all four main native targets; that does not establish compilation.

Installed Xcode metadata observed on the host is 27.2 (25400.63.5). An iPhone and Apple TV were visible to devicectl read-only earlier in the session. No admitted Apple Forge executor or resulting qualified native build/device receipt has been established. Do not install or claim a native release from this source checkpoint.

Required real-device acceptance after an admitted Apple build:
1. Launch signed out, open Music on this device, import a rights-cleared supported file from Files, and play it.
2. Relaunch with networking disabled and play the retained file and queue.
3. Exercise duplicate import, unsupported/protected media refusal, storage pressure, provider cancellation, and app interruption during import.
4. Switch between local and authenticated-server playback and sign out during preparation; verify one audio owner and no credential/reporting crossover.
5. Remove one selected local copy, verify the original and other local items remain, and verify backup/file protection attributes.
6. Complete the separate pinned invitation/passkey pairing-to-library journey before any deployment claim.

## Apple references

- AVFoundation asynchronous protected-content property: https://developer.apple.com/documentation/avfoundation/avpartialasyncproperty/hasprotectedcontent
- AVFoundation asynchronous playable-content property: https://developer.apple.com/documentation/avfoundation/avpartialasyncproperty/isplayable-45h5v
- Files picker security scopes: https://developer.apple.com/documentation/swiftui/view/fileimporter(ispresented:allowedcontenttypes:oncompletion:)
- File coordination for scoped access: https://developer.apple.com/documentation/uikit/providing-access-to-directories
- Documents and backup behavior: https://developer.apple.com/documentation/foundation/using-the-file-system-effectively

This feature was authored for the public Bluefin source. It does not copy private Omnidia implementation code or claim an Omnidia private service's authority.

## Private artwork routing

The generic native artwork entry point refuses `owned-` item IDs before any cache/network lookup. Local rows and Now Playing therefore use local placeholder artwork; the private content digest is not sent to a connected server to search for a cover. The prefix only prevents network routing. It never grants file access or sharing rights.
