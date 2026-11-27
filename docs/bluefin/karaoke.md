# Karaoke preparation

Bluefin uses two authorized supplied PCM stems on the existing AVPlayer clock.
The Sing button in MusicLyricsView requests a package through the Rust core,
downloads integrity-checked stems, and calls enableKaraoke. It shows preparation,
unavailable/error, cancellation and retry states. No media goes to a cloud API.
Supplied stems are never described as inferred vocals.

## Admission and HTTP contract

Register bluefin_karaoke::register on the application Router<AppState>. The API
crate needs a path dependency on bluefin-karaoke and a handlers module declaration.
The workspace needs the new crate member. No new AppState field, constructor,
service, account or production setting is required.

GET /Bluefin/Karaoke/{item UUID} returns camelCase Package JSON. GET
/Bluefin/Karaoke/{item UUID}/{revision}/Stems/accompaniment and /Stems/lead-vocal
return complete audio/wav bodies. Both require the existing current user session,
visible Audio item, enable_media_playback and enable_content_downloading. API keys
are rejected. Every request reloads library visibility and current viewer policy
through the existing auth/library services. A client-selected user or path never
selects the source. Responses use Cache-Control: no-store. Missing/invalid imports
return 404, changed revisions 409, policy failures 403, overload 503. There is no
inference job endpoint and no fabricated server processing progress.

The manifest contains itemId, revision, durationSeconds, processing=suppliedStems,
accompaniment and leadVocal descriptors containing bytes/sha256, and the legacy required timeline. The optional `timingKind` marker supports explicit `none` with no timeline, or `word`/`syllable` with a validated timeline. Absent marker plus absent timeline remains invalid; see the [preparation contract](karaoke-preparation-contract.md#lyric-precision).
It contains no paths, credentials, supplier rights declaration or model settings.
The revision is the SHA-256 of the exact admitted private manifest. A stem request
must name that revision. Download responses do not redirect. Server admission
reads both bounded stems and hashes the library source on every request, trading
CPU/I/O for stateless fresh admission. Two blocking admissions can run concurrently;
excess requests fail immediately. This is not a server-wide network memory quota.

## Private supplied-stem import

An authorized operator stages one directory named by the canonical dashed item UUID
inside application_paths().internal_metadata_path()/bluefin-karaoke. This is
private runtime metadata, not checked-in product configuration. The directory has
exactly these required regular files: manifest.json, accompaniment.wav,
lead-vocal.wav. Publish a complete directory atomically while no preparation is
in progress. Do not let library users or HTTP clients write this directory.
There is deliberately no HTTP endpoint accepting filesystem paths or uploads.
The operator must establish actual rights to the source, stems and timed lyrics;
a nonempty rights reference records that admission, it cannot prove legal rights.

The strict private manifest has these fields, all required:

```json
{
  "itemId": "00000000-0000-0000-0000-000000000001",
  "sourceSha256": "<sha256 of the exact library Audio source bytes>",
  "durationSeconds": 1.0,
  "rightsReference": "<private operator admission record>",
  "accompaniment": {"sha256": "<sha256>", "bytes": 16044},
  "leadVocal": {"sha256": "<sha256>", "bytes": 16044},
  "timeline": {
    "offsetSeconds": 0.0,
    "lines": [{"id": "one", "voice": "lead", "tokens": [
      {"text": "La", "start": 0.0, "end": 1.0}
    ]}]
  }
}
```

Unknown private manifest fields are rejected. Source hashing is limited to 2 GiB;
manifest JSON to 1 MiB; each stem to 64 MiB. Every path component is opened with
no-follow semantics, including intermediate directories and the library source.
Non-regular files, symlinks and traversal components fail closed. Secure admission
currently supports Unix hosts; other hosts return Unsupported rather than use a
weaker filesystem check. Source hash, item UUID and duration must agree with the
current library item. Duration tolerance is 50 ms. Both stems must have identical
sample rate, channel count and frame count. Accepted files are canonical 44-byte
header little-endian PCM16 WAV, mono/stereo, 8–192 kHz, positive duration <=4 hours,
without metadata/extra chunks. The byte limit usually imposes a shorter duration.
This intentionally narrow format rejects arbitrary compressed audio or external
references. Hashes bind an operator's declared source/stem relationship; software
cannot authenticate artistic provenance or rights from waveform data alone.

Timed tokens must be supplied. Voices are lead, partner, together and backing.
Lines can overlap for duets. Tokens in a line cannot overlap. Positive offset
means delayed lyrics. No timing is inferred from line-only text. Lyrics use each
word/syllable's supplied interval for continuous horizontal fill, measured token
wrapping, duet alignment, accessible line labels, follow/browse and seek. Very
long multi-line tokens and right-to-left fill need visual qualification.

## Native registration and lifetime

Parent adds `mod karaoke;` to the Apple Rust core and dispatches both
`karaokeManifest` and `karaokeStem` to karaoke::call(&api, operation).
No new Rust core dependency is required. Manifest input is itemId. Stem input is
itemId, revision, role, sha256, bytes and destination. Destination is a native
app-owned temporary filename, never sent to the server. Rust uses the existing
Api origin/auth/transport rules, disables redirects, creates a new mode-0600 file,
streams within the descriptor byte bound, checks SHA-256 and fsyncs. A failed
transfer removes its partial file. Native code creates the unique mode-0700
directory and owns it before calling Rust, so cancellation does not lose its owner.
RustBridge waits for synchronous FFI completion before surfacing cancellation;
a blocked transfer can take its bounded 120-second timeout to settle.

Include KaraokePreparation.swift, KaraokePreparationModels.swift and
KaraokePreparationTests.swift in project generation. Preparation binds the current
item/server/user/token across every suspension. Once enabled, the directory lease
is retained by an associated object on the prepared AVPlayerItem. It therefore
survives view dismissal, and expires after stop, source replacement, account
teardown or disable releases the final AVPlayerItem consumer. Failure/cancellation
before enable releases the task's lease after synchronous FFI returns. The existing
PlaybackController already stops and clears karaoke on stop; parent must preserve
its account-change stop integration. Process crashes can leave orphan temporary
directories for the OS to reclaim; there is no crash-recovery cache claim.

## Server separation engine

The server-only [Rust engine](karaoke-engine.md) runs the official Spleeter 2-stem
checkpoint with Candle and RustFFT. Its archive matches the publisher's release
checksum. The authors' [pinned paper](https://github.com/deezer/spleeter/blob/6f2c8a2e283914faa544631430f65b793f1b1218/paper.md)
explicitly licenses both source and pretrained models under MIT. Preserve the
engine's notices and provenance; permission for the model does not license audio
or lyrics. No model or inference runtime belongs in the native client.

CPU inference, publisher-graph and waveform parity, cancellation and resource
bounds have component qualification. Authenticated server job orchestration and
integrated native acceptance remain separate gates. The engine currently accepts
at most 300 seconds of prepared stereo PCM at 44.1 kHz. Its published model masks
suppress frequencies at and above 11.025 kHz and attenuate the opening window;
generated stems must be identified as processed audio, not an untouched lossless
source or Apple Music Sing parity. Exact authorized word timing is still required.

Private imports distinguish `suppliedStems` from a validated `modelSeparated`
processing receipt. Public processed packages use `modelSeparatedStems` and a
public model reference, while original-source and prepared-PCM digests stay private.
An import receipt never replaces current source, account and library authority.

## iPhone microphone on Apple TV

No private Apple Sing API is used. Apple documents a supported public alternative
in [Supporting Continuity Camera in your tvOS app](https://developer.apple.com/documentation/avkit/supporting-continuity-camera-in-your-tvos-app).
It requires compatible Apple TV 4K hardware and tvOS 17+, outside the OS15 baseline.
This is a capture path, not evidence of equivalent Sing latency or monitoring.

A separate parent-owned integration can add an availability-guarded tvOS17 capture
coordinator: explicit connect/disconnect UI through AVContinuityDevicePickerViewController and its delegate, or the SwiftUI
continuityDevicePicker modifier. Handle the returned AVContinuityDevice, monitor
AVAudioSession.isInputAvailable and availableInputs, then configure a bounded
AVAudioEngine monitor. Apple's sample uses playAndRecord with voiceChat and voice
processing; capture-session video input is a separate camera concern, not a
requirement to record or display video. Contract states should be unavailable,
selecting, permissionDenied, connectedMuted, monitoring and interrupted. Default to
muted; require deliberate user gain enable; stop on disconnect, interruption,
account change, app teardown and original-audio restoration. Do not record or send
samples to a server. Keep bounded buffers, cap gain and prove safe routing, feedback,
latency and interruption recovery on devices. OS15 keeps the current wired iOS
monitor and a truthful unavailable TV message. Device sessions and changes to the
existing microphone implementation belong to parent; this lane did not run them.

## Verification scope

The crate contains original synthesized sine-wave fixtures and rejection tests.
The Rust transport tests use a loopback fixture server to check authentication,
origin route, integrity cleanup, forbidden responses, redirects and owner mismatch.
HTTP handler tests exercise real auth/policy gates. Native tests check package bounds,
lease retention/deletion and the existing offline AVFoundation mix. These do not
qualify installed playback, separation quality, real microphone latency, OS15 device
behavior or end-to-end production deployment. Those remain parent integration gates.
