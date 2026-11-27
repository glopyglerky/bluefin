# Server karaoke preparation contract

This server feature separates an authorized local audio source with the original Spleeter 2-stem model. Opening Sing may submit an asynchronous preparation request under the current account's existing media permissions. Vocal reduction works without a separate timing package: clients keep using ordinary authenticated lyrics, including real line timestamps, while the prepared audio supplies the playback clock. Independently supplied word or syllable timing remains supported. Separation generates no lyrics or timings. Model output is lossy separated PCM; it does not claim original/lossless or full-band quality.

## HTTP

All requests require a current normal `Authorization: MediaBrowser ... Token="..."` device session. API keys, query-token and legacy-header fallback, request extensions, caller user IDs and opaque markers supply no authority. Current token/device, account, network/IP, remote-access, schedule, playback, download, audio-transcoding, library, tag, parental and captured-source classification checks apply. Revoked or unauthorized requests use ordinary 401/403/404 responses.

- `POST /Bluefin/Karaoke/{itemId}/Preparation`: `{ "requestId": "non-nil UUID", "mediaSourceId": "optional actual source ID" }`.
- `GET /Bluefin/Karaoke/{itemId}/Preparation/{requestId}`.
- `DELETE /Bluefin/Karaoke/{itemId}/Preparation/{requestId}`.
- Existing manifest `GET /Bluefin/Karaoke/{itemId}` and `GET /Bluefin/Karaoke/{itemId}/{revision}/Stems/{role}` remain; optional query `MediaSourceId` selects the exact source. Roles are `accompaniment` and `lead-vocal`.

Preparation responses contain `itemId`, `jobId` equal to the request UUID, actual `mediaSourceId`, `state`, `retryAfterMilliseconds: 1000`, and optional closed `reason`. States are `queued`, `preparing`, `ready`, `unavailable`, `failed`, `cancelling`, `cancelled`. Reasons are `timingRequired`, `durationUnsupported`, `unsupportedSource`, `serverDisabled` (reserved), `busy`, `preparationFailed`. No internal errors or paths are returned.

Explicit protocol clarification: a DELETE-before-POST tombstone has `mediaSourceId: null`, since no source has yet been selected. Accepted POST jobs resolve the actual source before returning. Disabled preparation endpoints return 404 for feature detection; authenticated `/Bluefin/Capabilities` additionally advertises `karaokePreparation.enabled: false`. The capability also reports `requiresSuppliedTiming: false` and `supportsStemOnly` equal to the service's enabled state. Existing supplied imports remain usable with preparation disabled.

Request IDs are idempotency identifiers scoped to the real account/session/device. Other scopes cannot inspect/cancel them. Conflicting source reuse is 409. The service admits one heavy job and at most two waiting jobs; overload returns `unavailable/busy`. History and tombstones are bounded to 256 records and ten minutes after completion. History exhaustion refuses new requests. A bounded private atomic journal preserves stopped IDs across restart for their remaining TTL; restart treats retained IDs as cancelled, so a client needs a new UUID for new work. This prevents delayed POST resurrection. Client stop does not delete a completed cache owned by that account. Approved devices of one account may reuse it after current authorization; other accounts do not discover or reuse it. `cancelling` becomes `cancelled` only after actual worker/cleanup join; `ready` means execution permits have returned.

All manifest, stem and status responses are `no-store`. Prepared manifests add `mediaSourceId`, `processing: "modelSeparatedStems"`, `processingReference: "deezer/spleeter/2stems@v1.4.0"`. Legacy private supplied packages may omit the new fields and retain `processing: "suppliedStems"`. Their public API response includes the exact currently admitted `mediaSourceId`, so a client can bind Sing to the edition already playing without weakening legacy import validation. Original/PCM/authority/rights/timing digests and references remain private. Every manifest/stem read reopens current source authority, validates the exact original digest and current generation, and rechecks after reading. A changed source has no stale cache fallback.

## Lyric precision

The optional public/private manifest field `timingKind` distinguishes audio availability from lyric precision:

| Marker | Required timeline | Client lyric display |
| --- | --- | --- |
| Absent (legacy) | Valid nonempty timeline | Existing timed-package behavior |
| `"none"` | Absent or null; a supplied timeline is invalid | Ordinary `GET /Audio/{itemId}/Lyrics` |
| `"word"` | Valid word timeline | Supplied word boundaries |
| `"syllable"` | Valid syllable timeline | Supplied syllable boundaries |

New model output always includes an explicit marker. Missing/corrupt legacy timelines cannot silently downgrade into stem-only playback. An ordinary line-timed LRC remains line-timed; the server does not invent per-word boundaries or duplicate its text into the stem package. A client should use its prepared-audio clock for existing lyrics, seeking and progress.

Public `durationSeconds` describes the admitted stems' decoded frame count and
sample rate. Timeline admission uses that PCM duration. Catalog/container duration
is checked separately with its existing metadata tolerance; its rounding must not
extend playback or lyric timing. Native admission compares the decoded stems with
the manifest to within one sample before replacing the current player item.

## Trusted process-start settings

Without `BLUEFIN_KARAOKE_CONFIG`, no model/cache is opened and preparation is disabled. If set, it names an absolute operator-owned non-symlink JSON file, for example:

```json
{
  "modelArchive": "/srv/bluefin-models/2stems.tar.gz",
  "cacheRoot": "/srv/bluefin-cache/karaoke-model-owned",
  "timingRoot": "/srv/bluefin-metadata/karaoke-timing",
  "cacheBytes": 536870912,
  "cpuThreads": 3,
  "deadlineSeconds": 600
}
```

Paths are never accepted from HTTP. Model, timing and cache scopes must be separate. Timing/cache directories are effective-user-owned mode 0700; all path components are opened without following symlinks. The cache is dedicated, marked for generated-storage ownership only, and exclusively locked. Unexpected files, symlinks or unmarked nonempty directories cause refusal and are preserved. No supplied package is overwritten or swept. Invalid enabled configuration/model/thread settings fail startup closed. Set both `RAYON_NUM_THREADS=3` and `CANDLE_NUM_THREADS=3` **before process startup** for this example. No global environment mutation occurs; the engine validates the actual Candle pools. CPU thread range is 1..4; deadline range is 1..600 seconds.

Only the original official `deezer/spleeter` v1.4.0 archive SHA256 `f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692` is accepted. Engine 0.1.0 uses CPU Candle 0.11.1 and RustFFT 6.4.1. Product builds leave the optional `qualification` feature disabled. No model downloader, TensorFlow runtime, Python, native inference or inference subprocess is used. The existing server's platform/native dependencies and ordinary catalogue probes are separate compatibility dependencies.

## Optional supplied timing

An empty trusted timing directory permits ordinary stem-only preparation. No per-song operator attestation is required for this path: current authenticated account policy and exact source identity authorize the operation, and are checked again before publication and every read. The private `rightsReference` field on new model output is an `account-policy-v1:` receipt reference to that actual admission; it is not a legal-rights assertion or an authorization token. Any supplied timing's existing rights reference is preserved separately in the private preparation receipt.

For word or syllable lyrics, the private operator-owned `timingRoot/{item UUID}/{actual simple source UUID}.json` contains:

```json
{
  "itemId": "00000000-0000-0000-0000-000000000001",
  "mediaSourceId": "00000000000000000000000000000001",
  "sourceSha256": "SHA256 OF ORIGINAL ENCODED BYTES",
  "rightsReference": "Private admission reference for the supplied timed lyrics",
  "timingKind": "word",
  "timeline": {
    "offsetSeconds": 0,
    "lines": [{"id":"line1","voice":"lead","tokens":[{"text":"Example","start":0.1,"end":0.9}]}]
  }
}
```

`timingKind` in a supplied timing file is `word` or `syllable`. Item, selected source, original hash, private provenance reference and exact finite timeline must match. Only an absent optional file selects stem-only mode. A present but malformed, inaccessible, unsafe or source-mismatched file returns `unavailable/timingRequired`. The trusted root is revalidated on each access. Timing changes during preparation prevent publication; adding or removing a package changes the private cache identity. No guessed or proportional timeline is produced. There is no microphone/upload endpoint, provider extraction or inferred entitlement.

## Ownership and bounds

The real Core source pin retains the opened source and captured original library classification. A narrow `MediaSourceManager::current_local_audio` accessor revalidates the complete item/source/version/stream/attachment/ancestry/library generation and current opened identity without changing TV's frozen dispatch semantics. `AuthService::authenticate_current_session` reads actual stored device/account/policy rows without token or DTO caches. These are the parent merge hooks; neither is an authorization marker or cached grant.

Preparation snapshots and hashes at most 128 MiB of encoded bytes into immutable owned memory, checking file length/mtime/ctime around the read. Symphonia 0.5.5 decodes only preflight-bounded PCM/float RIFF WAV, native FLAC and finite Layer III MP3; MP4/AAC/ALAC/Vorbis/AIFF are unavailable. Chunk/metadata lengths are checked before demuxer allocation; Rubato 0.16.2 performs anti-aliased resampling with delay compensation. Input is restricted to known duration/frame count, mono/stereo and 8..48 kHz. Decoder buffers are checked at 65,536 frames, packet data at 4 MiB, total packets at 300,000 and total decoded frames at 300 seconds. Unsupported/encrypted/live/remote/disc/unknown-duration inputs fail closed. More than 300 seconds is explicitly unavailable, never silently truncated. Canonical engine PCM is stereo 44.1 kHz PCM16, at most 52,920,044 bytes. Common-codec support comes from Symphonia; the integrated qualification fixture specifically exercises a real encoded 48 kHz mono FLAC decode/resample, plus a focused WAV resampling test, not every codec/container combination.

The original encoded digest and prepared PCM digest are separate private receipt fields. Cache identity includes the authenticated account UUID, real item, selected source, logical/file generation, original digest, model/runtime version and optional timing hash. Account policy admission is also bound into each generated private manifest, so prepared public revisions are distinct across account owners even for identical authorized sources. Neither the cache index nor private source/account hashes are exposed. This storage is local to the instance and does not implement a global cross-account content index. Before atomic NOREPLACE publication, the server rechecks current policy, original bytes and timing. Opened engine outputs are length/hash/PCM/duration validated; task staging cleanup is explicitly joined. A cleanup failure poisons further preparation and preserves reconciliation debt. Disk reservations cover both engine output and cache copies plus metadata; the trusted cache budget is 2 MiB..2 GiB. Cache TTL is one hour. Reads have a separate two-request bound, and prepared-cache reads share the small admission domain. Application memory/CPU/disk bounds are not kernel quotas.

Queued and active jobs revalidate authority on bounded 250 ms checkpoints. Cancellation/deadline is cooperative between decoding/model kernels; one synchronous kernel or filesystem syscall cannot be immediately preempted. Source pins and admission permits remain with the actual blocking worker through cleanup. Shutdown stops enqueueing, cancels queued/active work, joins it and purges completed owned caches before Core/DB teardown. Restart safely removes recognized incomplete generated directories, preserves unexpected contents and retains stopped request IDs.

## Compatibility and qualification

This is a Bluefin extension; no Jellyfin route or lyric DTO parity behavior changes. Legacy supplied packages continue to require their timeline and private admission record. The original model-backed server qualification now covers stem-only FLAC decode/resample with an ordinary LRC, unchanged line timestamps without invented word cues, same-account device reuse, distinct-account preparation isolation, legacy imports, exact-source word packages, policy/token revocation, cancellation/join, bounded storage and restart cleanup.

Browser playback lease create/renew responses expose the exact planner-selected mediaSourceId from their retained source pin. A Sing client must bind preparation and the returned package to that active source; an absent source must not be replaced by a guessed first edition. This browser metadata does not change the strict TV v1 lease wire response or grant additional access.
