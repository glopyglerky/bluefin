# Server vocal separation engine

`crates/bluefin-karaoke-engine` runs the official Deezer Spleeter 2-stem U-Nets on CPU using Candle Core 0.11.1 and RustFFT 6.4.1. It produces actual vocal and accompaniment WAVs and is now a root workspace member. The authenticated opt-in server integration is documented in [karaoke-preparation-contract.md](karaoke-preparation-contract.md). The component qualification and original integration requirements below preserve the preceding standalone engine delivery; native-client and whole-product acceptance remain separate.

## Model and permission evidence

Only the original publisher asset is accepted:

| Input | Pin |
| --- | --- |
| Publisher | `deezer/spleeter`, release `v1.4.0` |
| Tag commit | `556ef2121492d72398a988af1b38b55176f5973a` |
| [Official 2stems.tar.gz](https://github.com/deezer/spleeter/releases/download/v1.4.0/2stems.tar.gz) | SHA256 `f3a90b39dd2874269e8b05a48a86745df897b848c61f3958efc80a39152bd692` |
| Publisher checkpoint data | SHA256 `7747f9fd2c782306dbec1504360fbb645a097a48446f23486c3ff9c89bc11788` |
| Publisher checkpoint index | SHA256 `55661a09f79c86071fc7077b44b4cf1f01b6d82bbbb83ed94cb68a2f9944e378` |
| Publisher MetaGraph | SHA256 `6e1f6d86a22bb452a58cb20e3de87b416f8a79e626ca223e20f763ab2d21ec95` |
| Pinned model configuration | SHA256 `c0cdb7c3c3b9ecdf378cda45f8a6254ddbc50f1035c24072a21df52d8890788d` |
| Pinned U-Net source | SHA256 `862837c9f60cd025a5949ebfb904b76b60bad35a0ba094b71364a7c8dbba9415` |
| Pinned prediction builder | SHA256 `d34d6f6a9c1ea6e36906ad402e3c80001ac3b5994d6e85a866fc70c62a5af406` |

The archive matches the official release `checksum.json`. The authors' [pinned paper](https://github.com/deezer/spleeter/blob/6f2c8a2e283914faa544631430f65b793f1b1218/paper.md) explicitly covers both source and pretrained models under MIT; the local `paper.md` SHA256 is `32fd4a4d55bc454f9fe5340d426819436da0c9f0151b7b8ebee0a37d95759983`. Its exact commit is verified in `paper-source-commit.json`. The [JOSS paper](https://doi.org/10.21105/joss.02154) identifies this work. Keep `notices/spleeter-LICENSE`, `notices/publisher-provenance.json` and the complete publisher-input manifest with model distribution. This permission evidence does not license any user's audio or represent training-data rights.

The model is not converted. `checkpoint.rs` reads the hash-pinned archive in memory, validates TensorBundle table-block CRCs, checks each selected tensor's exact shape and finite F32 values, and loads the original tensor bytes. It never extracts checkpoint paths, evaluates Python/pickle, imports a GraphDef in production, or downloads a model. The TensorFlow weight layouts are permuted to Candle's channel-first layouts without quantization.

## Public Rust contract

Construct one `Engine::load_official_archive(archive_bytes, Config, &Cancellation)` on a server-owned blocking worker. `Config` is trusted configuration, not request JSON. Set `RAYON_NUM_THREADS` and `CANDLE_NUM_THREADS` to `Config::cpu_threads` before starting the server process (default `3`). The engine never mutates process-global environment. It verifies both settings and the actual persistent Candle Rayon/barrier pool sizes; missing, incompatible or previously initialized mismatched pools fail closed with `ThreadConfig`. A process-wide RAII lease permits one engine instance; initialization errors release the lease. `process` requires `&mut Engine`, so the parent supplies one bounded queue and holds the worker/resource permit until execution and cleanup finish, including after HTTP cancellation.

`Input<'a, A>` accepts immutable, explicitly preadmitted PCM WAV bytes, their server-pinned expected SHA256, and an opaque owned attachment `A`. The engine verifies the complete PCM digest and returns `A` unchanged in `Processed<A>`. An attachment is a transport mechanism, not proof of authority. The parent must carry its real authenticated authorization, original source digest/generation, item identity, rights reference and preparation receipt in its existing source-pin/task object. It must check current authorization and the pinned original source before preparation and again before admitting the result. The PCM digest identifies the prepared audio, not an encoded original file.

Accepted input is deliberately narrow: a canonical 44-byte-header, little-endian signed PCM16 WAV, stereo, 44,100 Hz, with exact RIFF/data lengths and no extra chunks. Parent-owned decoding/resampling is a separate preparation hook. Unsupported formats, mono, wrong sample rate, partial headers/data, oversized audio and digest mismatch fail closed. No input URL or filesystem path is accepted.

Pass a `StagingRoot::from_opened_directory(File)` capability for a private, effective-user-owned `0700` directory on the build/task filesystem. The parent securely opens every directory component without following symlinks. The engine creates a random `0700` task directory, writes fixed `0600` filenames with descriptor-relative `O_EXCL | O_NOFOLLOW`, syncs complete files and atomically renames the whole directory with Linux `RENAME_NOREPLACE`. It never overwrites an existing destination. Filenames do not incorporate a client item ID or path.

`Processed<A>` contains `provenance` and `files`. Use `files.open_stem(Stem::LeadVocal)` and `files.open_stem(Stem::Accompaniment)` to obtain opened descriptors; revalidate output digests/lengths while importing those descriptors. `open_provenance()` opens the private JSON receipt. Do not resolve a caller-selected path from `directory_name()`. The directory exists while its `PublishedTask` handle is alive. Call `files.remove()` after parent admission/copying to observe cleanup errors; dropping the handle also attempts cleanup. Failure removes only the three known generated files and owned directory. Unexpected extra files are preserved and surface a cleanup error. `Error::Cleanup` preserves both the primary failure and cleanup failure for reconciliation.

The default bounds are 300 seconds (52,920,044 input bytes), one model/job per process, three workers in Candle's Rayon pool plus two persistent barrier helper threads, and a 600-second task deadline. There is no additional engine-owned Rayon pool. Trusted configuration permits at most four Rayon workers and three barrier helpers, 300 seconds and a 3,600-second deadline. Candle's pools persist for the process lifetime. Two output WAVs each have exactly the input frame count and are below the parent's 64 MiB stem limit. One 512-frame neural patch is evaluated at a time; FFT storage does not grow with track duration. Input, waveform/output storage and model tensors have explicit fixed maxima. These are application bounds, not kernel hard memory/CPU quotas; the parent should keep its existing resource admission/pressure controls.

Cancellation is checked while reading/decompressing the model, hashing/preparing PCM, between FFT frames and neural layers, during encoding/writes, and before/after publication. An individual tensor kernel or filesystem sync is synchronous and cannot be preempted by the token. A deadline uses the same cooperative checkpoints. Outputs reject non-finite samples, out-of-range PCM, malformed dimensions and incomplete lengths instead of silently publishing them.

## Exact processing

The Rust graph preserves raw convolution skip connections, raw sixth-convolution decoder input, the unused sixth encoder BN, encoder BN-before-LeakyReLU(0.2), decoder ReLU-before-BN, inference-only dropout omission, BN epsilon 0.001, NHWC-to-NCHW tensor layout, stride-2 asymmetric SAME padding/cropping and the last dilated convolution. Two independent networks predict magnitudes. The publisher's squared-magnitude ratio masks use epsilon `1e-10`.

DSP matches the publisher prediction graph: 4,096-point periodic Hann STFT, hop 1,024, end padding, zero-padded 512-frame patches, 1,024 trained frequency bins, `mask_extension=zeros`, inverse Hann overlap-add and compensation `2/3`, cropped to original duration. Frequencies at bin 1,024 and above (11,025 Hz and above) are suppressed in both stems; the publisher's leading window also attenuates the beginning. Full-band bit-exact mixture reconstruction is therefore not an acceptance claim. PCM16 adds normal quantization error. No multichannel Wiener filtering, automatic alignment, lyrics generation or loudness normalization is claimed.

## Original parent integration contract

The server implementation of these requirements is described in the preparation contract above. This historical component checklist does not claim native-client or live-installation acceptance.

1. Import this crate into the parent build without copying its standalone target/lock into the parent workspace. Resolve and audit the parent's combined lock while preserving the tested Candle/FFT pins and default CPU features. Keep `qualification` disabled in the server.
2. Add a server-owned preparation queue using the real source pin and current user policy. Keep task cancellation and resource permits attached through completion; reject overload. Prepare authorized canonical PCM in Rust or the parent's accurately disclosed existing media compatibility path.
3. Add processed-stem admission beside `bluefin_karaoke::admit`. Existing `Import` requires `item_id`, original `source_sha256`, `duration_seconds`, `rights_reference`, both stem digests/lengths and a validated `Timeline`, and always emits `processing="suppliedStems"`. Generated output must instead be recorded as `modelSeparatedStems` with the model/runtime/PCM and original-source-generation receipts. Never pass this output through supplied-stem admission or label it original/lossless.
4. Compose a real authorized supplied `Timeline`, validated against the exact output duration. Word/syllable timing is a separate remaining gate when such timing is absent. This engine produces no timing and makes no automatic-alignment or Apple Music Sing parity claim.
5. Update the parent public manifest and Swift validators to explicitly accept and display the processed provenance. Keep rights references, private source/PCM hashes, task capabilities and authorization internals out of public responses. Preserve current policy/visibility checks on every manifest/stem read, revision binding, no-store behavior and generation invalidation.
6. Qualify authenticated end-to-end parent behavior and cancellation/restart lifecycle before any integrated acceptance or deployment. This lane performed no live service, UI/device, production account or deployment action.

Reference inputs inspected from the retained parent snapshot:

| File | SHA256 |
| --- | --- |
| `crates/bluefin-karaoke/Cargo.toml` | `8c2626eb77c2abd5229909c27041086ab762e126633372b07caef660e034ed9d` |
| `crates/bluefin-karaoke/src/lib.rs` | `c1cd6ee3a6761c90162510d945ffa6eb2fc42481fc769d48bca46060ffc69661` |
| `crates/ferrofin-api/src/handlers/bluefin_karaoke.rs` | `aaf92330a1b89f44c49c6a05b54aea7068335980ef1fe93d864ad366a31af5dd` |
| `apps/ferrofin-server/Cargo.toml` | `9df8e3af57e5128139d05a0486f62169429d0d47b7e9a699ece8f64a93678b28` |

## Reproduction and dependency disclosure

Run Cargo with this crate's explicit manifest, `--locked`, a bounded job count, `CARGO_TARGET_DIR` and `TMPDIR` under the assigned generated-output directory. Set both thread variables described above to `3` for official-model tests and qualification. `cargo test --release` runs fast format/security/DSP boundary tests; the ignored official-model API test requires `BLUEFIN_MODEL_ARCHIVE` and must be run explicitly. Run `cargo fmt --check`, and Clippy with all targets/features and `-D warnings`.

Build the offline `qualify` binary with `--features qualification --release`. Its `inspect ARCHIVE META_OUTPUT` command records tensor/member hashes. `fetch-reference DIR` retrieves the official PyPI TensorFlow 2.21.0 wheel and verifies its published SHA256 before reading only two named native libraries. `parity ARCHIVE REFERENCE_DIR` compares a deterministic dense magnitude patch against the publisher graph. `audio ARCHIVE REFERENCE_DIR OUTPUT_DIR` creates original synthetic music and checks full-graph waveform parity, output decoding, quality/reconstruction and cleanup over two neural patches. `bench ARCHIVE OUTPUT_DIR SECONDS` measures the CPU engine in a fresh process without loading the reference runtime. Retain logs, manifest hashes and small receipts.

The isolated reference path is Rust tooling using TensorFlow's stable C ABI from the official 2.21.0 wheel, not a Python installation or product runtime. It freezes original ReadVariableOp values to exact checkpoint constants, removes only unreachable nodes and device/colocation hints, and retains the original false learning-phase/default graph behavior. The reference wheel SHA256 is `e9d8da8dcab9650efb45f032ba70af2f016f907e6e0c6bda29dd101bba945406`. Its code/native libraries are upstream TensorFlow, not owned Rust computation. The optional downloader's TLS stack includes upstream AWS-LC C code. Neither reference library nor downloader is part of the default engine dependency graph.

The production target is `x86_64-unknown-linux-gnu`, CPU only, without CUDA/Metal/MKL/Accelerate or owned subprocess execution. Candle Core's default CPU graph also includes its upstream Rust tokenizer/ZIP utilities, although this engine does not call them. OS calls, libc/platform bindings and unsafe upstream tensor/SIMD kernels are compatibility dependencies; no whole-stack memory-safety or all-Rust-OS claim follows.

The initial Candle 0.10.2/ZIP graph failed RustSec audit and is not the delivery dependency set. Candle Core is pinned to 0.11.1, upstream source commit `59f8e903ca80a32ea7f8514b39748fc979e69e82`, with patched ZIP 9.1.3. The final lock audit has no known security vulnerabilities; it reports `RUSTSEC-2024-0436` for the unmaintained compile-time `paste` macro dependency. That warning is disclosed, not suppressed. Exact registry package checksums are in the standalone Cargo.lock; resolved features, build metadata and audit revision are in the private delivery receipts.

## Qualification

The engine includes official-model initialization, inference, cancellation,
thread-configuration and cleanup tests. Publisher-graph comparison uses
original synthetic audio and checks magnitude estimates, reconstructed PCM,
frame counts and mixture reconstruction separately. Run the tests against
the pinned official archive and record its digest, dependency lock, features,
resource limits and model provenance.

Measure latency and peak memory on the intended deployment hardware. CPU
benchmark results do not establish real-time service performance, general
music quality, GPU support or native client playback. Authorized timing and
end-to-end server/client acceptance remain separate requirements. Keep raw
benchmarks, host specifications, generated audio and operational receipts in
private output storage outside this repository.
