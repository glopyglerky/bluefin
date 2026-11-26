# Candidate integration paths

Intended repository location for this tree: `docs/legal/bluefin-clients/`.
Keep inventory, provenance and original texts together. No source license changes.

Apple: copy the Apple notices text plus its referenced `third-party` resources into `apps/bluefin-apple/Resources/Legal/`, register them in each applicable app target, and expose them from the existing About/Account interface. Apply Rust resources to iOS/tvOS/Catalyst only; Watch currently links no Rust core. Do not create a Vision distribution scope from unbuilt code.

Android: copy the Android notices text plus its referenced resources into `apps/bluefin-android/app/src/main/assets/legal/`; expose local read-only access from About/Account. Package assets after review. Inspect the final APK and AAB, including text encoding, notices accessibility and native-library inventory. Project generation and product integration remain outstanding.

`inventory.json` separates normal runtime, host procedural macros, Maven runtime artifacts and reference-only toolchain evidence. `build-only-inventory.json` is audit metadata, not a shipped-package assertion. Notice text files are deduplicated by exact SHA256 bytes; distinct upstream copyright/NOTICE text is retained. Official standard-library HTML must accompany the plain text, since it contains additional original attributions.

Local vendored changes: noq-proto 1.4.0 has assembler compaction/retention repairs; netwatch 0.19.3 and netdev 0.46.3 have Apple interface/platform support patches. Preserve their original licenses and ship accurate modification/source disclosures with the corresponding source. Package version alone does not identify these modified copies.

SDK and host-only components are separate: Xcode/Apple SDK frameworks, OS Security/SystemConfiguration/Network, iconv/resolver/System libraries, Android SDK/platform APIs, JDK, Gradle and AGP are build or host interfaces. They are not copied wholesale as client dependencies. Swift system runtime linkage is a platform dependency; separately inspect any future embedded runtime libraries. No external model, server inference stack, FFmpeg, HLS JavaScript player or Spleeter model license is inferred from unused repository directories.

Review UNRESOLVED.md before distribution. This candidate provides notice evidence, not legal clearance or a store submission decision.

## Recovered evidence and compact indexes

Use NOTICE-INDEX.html as a resource entry point, or consume indexes/*.json from the native legal screen. Each JSON lists runtime candidate packages and relative original-text resources for one accepted target; host macros are omitted. Paths resolve relative to the JSON file. Standard-library HTML selects 1.95.0 for iOS/tvOS and 1.99.1 for Catalyst/Android. Preserve that original HTML and its links. Watch has no Rust linkage; Vision is not an accepted built Rust scope.

Keep first-party/LICENSE and first-party/README.md verbatim with the attribution. The repository-root files themselves are not replaced by this docs tree. The current BuildCore source has been inspected: default-feature locked release library builds, explicit remaps, iOS/tvOS 15.0 floors and explicit later-platform 26.0 floors. This source inspection does not prove a historical archive's build command.

The delta manifest is an allowlist under docs/legal/bluefin-clients only, with preimage and new hashes. Verify each preimage before applying changed files. Added legal resources must be copied with their exact bytes. Do not replace unrelated repository files. Parent owns product integration, corresponding-source route and final packaging acceptance. Original legal text is supplemented, not relicensed; recovered licensing explanations are not silently promoted to full grants.
