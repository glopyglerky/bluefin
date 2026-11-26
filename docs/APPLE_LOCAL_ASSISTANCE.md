# Apple local assistance

Source checkpoint, 2026-11-26. The implementation and tests are authored. No Rust compilation, Swift compilation, simulator run or physical-device model run has qualified this slice yet. Source availability checks are not evidence of hardware support.

## Implemented behavior

Bluefin has three explicit local assistance actions. Search can select up to five of twelve already loaded, account-authorized catalog items from a user's description. A detail view can highlight up to three exact sentences from its supplied description. The music player can explain an enumerated playback state using supplied, redacted sentences. Each action requires a button press. Suggestions open through existing library navigation.

The description action is extractive. The model chooses sentence indices; Rust returns the original source excerpts. It does not generate a paraphrase. Playback explanations use predefined sentences and do not infer a cause from a generic buffering or failure state. A description can contain spoilers, so this UI makes no spoiler-free promise.

The new `bluefin-local-assist` crate owns input validation, prompt preparation, ephemeral slot identifiers, output allowlists, logical deadlines and completion consumption. It uses `#![forbid(unsafe_code)]`. Swift calls the public Foundation Models API through the app's existing JSON bridge. The existing bridge, system framework and transitive dependencies are outside that crate's safe-Rust claim. No new Rust-to-Swift FFI wrapper was added.

## Data and authority

Only selected title, description, year and runtime fields enter model context. Catalog IDs stay in Rust and become temporary slot names in the prompt. A local account-lifetime UUID fences delivery but is absent from the prompt. Provider IDs, external URLs, user playback history, account sessions, mesh invitations, QR codes, passkeys, tokens, network addresses and private logs are not supplied by the adapter.

Strict typed decoding rejects extra input fields. Additional text checks reject recognizable credentials, invitation markers, URLs, IP addresses and opaque token-shaped strings. These checks are defense in depth, not a universal secret detector. New callers must preserve the typed, authorized-source boundary and must never pass arbitrary logs or account objects as descriptions.

The model returns only catalog slots or sentence indices. Rust rejects unknown fields, unknown or duplicate slots, mixed output forms, invalid indices, repeated completions and stale or expired tickets. The model cannot grant account access, approve signup, invoke tools, change storage, choose network destinations or start playback. The app still owns those operations.

Every prepared request and accepted result carries `on_device`, `no_tools`, `no_training` and `ephemeral` provenance. There is no prompt transcript persistence or model-result logging in this path. The model uses the default guardrails. It has no custom tools, adapters, remote provider or Private Cloud Compute fallback.

Apple's current program agreement restricts using Apple-model output to train, fine-tune or improve another model, and requires rights to the supplied input. Bluefin applies that restriction to the model's choices as well as any derived display result. The Rust result-purpose gate refuses model training or improvement. Do not route these results into another model's datasets, fine-tuning, distillation, reward examples or improvement pipelines. See [the program agreement, sections 3.2(h) and 3.3.11(A)](https://developer.apple.com/support/terms/apple-developer-program-license-agreement/) and [Foundation Models acceptable use requirements](https://developer.apple.com/support/terms/acceptable-use-requirements-for-the-foundation-models-framework/).

## Bounded execution and cancellation

| Bound | Current value |
| --- | --- |
| Catalog candidates | 12 |
| Query | 512 UTF-8 bytes |
| Title | 200 UTF-8 bytes |
| Catalog description | 320 UTF-8 bytes per item |
| Selected full description | 2,800 UTF-8 bytes |
| Description sentences | 24, each at most 800 UTF-8 bytes |
| Instructions plus serialized prompt | 8,192 UTF-8 bytes |
| Requested generated response | 192 tokens |
| Accepted serialized output | 2,048 bytes |
| Returned selections | 5 items or 3 source sentences |
| Logical deadline | 20 seconds |
| Retained Rust requests | 4 maximum |
| Real in-flight framework calls | 1 globally in the app |

The Swift controller creates a fresh session for each request. Cancellation, account changes, view removal, backgrounding and timeout fence late delivery. The single global model permit remains held until the framework's awaited call returns, even if cancellation was requested earlier. Repeated requests cannot accumulate abandoned inference calls. A slow cancellation can keep assistance busy while ordinary UI remains usable. The app does not claim that requesting cancellation proves the OS has already stopped work.

The byte cap is not a token-count guarantee. Instructions, prompts, schema and output consume model context. This first adapter uses the base version 26 API and handles context or generation failures by retaining normal UI. It does not assume one context size for every model version. Apple introduced runtime token-count and context-size APIs with its 26.4 updates and documents further model changes in version 27. Version-specific prompt and device qualification is still needed. See [Foundation Models updates](https://developer.apple.com/documentation/updates/foundationmodels) and [single-turn generation guidance](https://developer.apple.com/documentation/foundationmodels/generating-content-and-performing-tasks-with-foundation-models).

## Platform boundary

| Platform | Source behavior |
| --- | --- |
| iOS and iPadOS 26 or later | Eligible only when the framework is present, the default model reports available and the current locale is supported |
| Mac Catalyst and macOS 26 or later | Same guarded provider path; Bluefin's Mac app is a Catalyst target |
| visionOS 26 or later | Same guarded provider path, pending target and hardware qualification |
| Earlier supported app OS versions | Ordinary UI, without local assistance actions |
| tvOS and watchOS | Local inference explicitly excluded; ordinary controls remain available |

The provider is specifically `SystemLanguageModel.default`. Apple's symbol documentation identifies this as the on-device model and lists its platform availability. Availability also depends on the device, region and model readiness. Generic Foundation Models framework availability must not be treated as evidence that this provider runs locally on Apple TV or Apple Watch. See [SystemLanguageModel](https://developer.apple.com/documentation/foundationmodels/systemlanguagemodel).

The framework can reject a request after the UI initially reported availability. Those failures display a fixed message without exposing model errors or transcript details. No alternative provider is selected.

## Upstream evaluation

The reviewed public Rust wrapper is [blacktop/fm-rs at a47ef7b22f627732f5433687da2b64e634173341](https://github.com/blacktop/fm-rs/tree/a47ef7b22f627732f5433687da2b64e634173341), under MIT. Its reviewed build configuration targets macOS and iOS, its binding includes unsafe C FFI and Swift implementation files, and it does not add a reviewed visionOS build path. It also offers optional Private Cloud Compute support.

Bluefin did not import that wrapper or its source. The app already has a Swift entry point and a Rust bridge; keeping one small public-framework adapter avoids an additional FFI implementation. The Rust validation owner is independently authored and inherits this workspace's GPL-3.0-only license. This decision does not claim that the entire application or all dependencies are safe Rust.

## Twenty-five grounded uses

These are product ideas with distinct prerequisites. Only entries 1 through 3 have source attached to UI in this checkpoint, and they remain unqualified by builds or hardware tests. The later entries are proposals, not implemented features.

### First actions, implemented in source

| # | Use | Boundary |
| --- | --- | --- |
| 1 | Select relevant titles for a natural-language preference | Choose only from up to twelve already authorized loaded candidates; return existing IDs through slots |
| 2 | Highlight a selected description | Return exact source sentences; no invented summary or spoiler-free promise |
| 3 | Explain a playback status | Use one typed state and predefined redacted sentences; no log access or diagnosis |

### Further read-only ideas

| # | Use | Prerequisite and limit |
| --- | --- | --- |
| 4 | Compare two selected descriptions | New bounded comparison input and source-excerpt output; no facts beyond supplied metadata |
| 5 | Show why a suggested title matched | Return cited source sentence indices rather than unsupported recommendation prose |
| 6 | Suggest mood labels for a title | Reviewed finite vocabulary and source evidence; labels do not alter server metadata |
| 7 | Explain an unfamiliar genre | Select from a maintained glossary; no arbitrary model knowledge treated as catalog fact |
| 8 | Explain supplied content ratings | Preserve provider ratings and account restrictions; the model cannot decide parental eligibility |
| 9 | Turn a viewing-time preference into filters | Rust performs duration arithmetic and validates every filter before the user applies it |
| 10 | Offer a simpler reading version of a description | A separate paraphrase contract, factual review and clear generated-text labeling are required |
| 11 | Explain listed audio and subtitle choices | Receive only approved track labels and format enums; the user changes the track |
| 12 | Find relevant library help | Select approved help article IDs; never turn a help answer into an action |
| 13 | Explain offline availability | Typed download-state input, without file paths, URLs or storage permissions |
| 14 | Explain download progress | Rust supplies deterministic totals and state; no model arithmetic or automatic retry |
| 15 | Explain an unsupported media entry | Redacted capability enums and curated help text; no raw scanner logs |
| 16 | Group a search result page | Fixed category vocabulary and known item slots; no extra catalog retrieval |
| 17 | Make a temporary shared viewing shortlist | Use only items visible to the current account and preferences explicitly supplied for this request |
| 18 | Highlight chapter descriptions | Requires authorized chapter text and known chapter IDs; no automatic seeking |
| 19 | Translate a supplied metadata excerpt | Requires input rights, supported language checks and a separately evaluated translation contract |

### Later ideas requiring additional capabilities or controls

| # | Use | Prerequisite and limit |
| --- | --- | --- |
| 20 | Recap an authorized transcript segment | Licensed or user-authorized transcript ingestion, bounded text and source references |
| 21 | Highlight dialogue already watched | Reliable playback boundaries and timestamped authorized subtitles; boundaries reduce exposure but do not guarantee no spoilers |
| 22 | Find a described moment in supplied chapters | Known chapter or segment IDs and authorized descriptions; normal user navigation performs the seek |
| 23 | Turn a spoken preference into a catalog query | A separate speech-recognition permission and data policy; Foundation Models is not assumed to transcribe audio |
| 24 | Read an accepted explanation aloud | A separate speech-synthesis and accessibility path; do not infer voice capabilities from text inference |
| 25 | Explain an accessibility setting from approved help | New typed settings/help input; show instructions and leave all setting changes to the user |

No proposed use may train another model with Apple-derived output or receive mesh/account credentials.

## Qualification handoff

Authored checks cover unknown IDs, extra authority fields, malformed or oversized output, duplicate completions, source-excerpt grounding, secret/network-shaped inputs, deadlines, clock rollback, cancellation, account changes and request capacity. Bridge tests check strict credential rejection and routing without an API session. Pure Swift tests cover availability policy, retained in-flight permits, metadata minimization, typed playback input and exact description forwarding.

The intended admitted Forge commands are:

```sh
cargo test --locked --offline -p bluefin-local-assist
cargo test --locked --offline --manifest-path apps/bluefin-apple/rust/Cargo.toml --features portable-apple-check local_assistance
```

The build owner must reconcile both lockfiles inside the admitted dependency workflow before running locked checks. The new crate requires only existing serde and serde_json packages. The Apple bridge now directly uses serde derive.

After generating the Xcode project from `apps/bluefin-apple/project.yml`, qualify the existing iOS, Catalyst, visionOS and tvOS schemes, including `LocalAssistanceTests` where the app test targets include that file. Check the older supported OS compile paths and the tvOS exclusion. Physical eligible-device testing must separately verify model availability, supported locales, cancellation latency, background/account-change fencing and acceptable source selection. No such device evidence is recorded by this checkpoint.

Implementation paths are `crates/local-assist`, `apps/bluefin-apple/rust/src/local_assistance.rs`, `Sources/LocalAssistance.swift`, `Sources/LocalAssistanceViews.swift` and `Tests/LocalAssistanceTests.swift` under the Apple app. UI attachments are in `UnifiedSearch.swift`, `BluefinApp.swift` and `MusicPlayer.swift`.

## Explicit sampling and authority

The native adapter requests `GenerationOptions(samplingMode: .greedy, maximumResponseTokens: ...)`. This makes sampling repeatable only for the same input, session state, and exact on-device model version. An OS/model update can change the result. It does not establish factual correctness or permission to act. The existing deterministic Rust ticket, candidate-ID, byte/token budget, and stale-generation checks remain authoritative; the model has no file deletion, resource allocation, storage donation, sharing, or provider-entitlement tools.

The installed Xcode 27.2 iPhone SDK declares the current `samplingMode` initializer available from iOS/macOS/visionOS 26.0 with back deployment before 27.0. This was verified by reading the SDK interface, not by compiling or running the app. Apple qualification is still outstanding.

Primary sources: [GenerationOptions initializer](https://developer.apple.com/documentation/foundationmodels/generationoptions/init(samplingmode:temperature:maximumresponsetokens:)), [greedy sampling](https://developer.apple.com/documentation/foundationmodels/generationoptions/samplingmode-swift.struct/greedy), and [WWDC25 Foundation Models deep dive, sampling](https://developer.apple.com/videos/play/wwdc2025/301/).
