# Noq assembler backport inventory

This is a modified copy of the published noq-proto 1.4.0 crate, not a new upstream release. Package identity remains 1.4.0. Only `src/connection/assembler.rs` differs from the original crate; this inventory is added. All other distributed files, including both licenses and original Cargo metadata, remain unchanged.

Original archive: https://static.crates.io/crates/noq-proto/noq-proto-1.4.0.crate

Archive SHA256: c01a35be3ed37558c8db69554419f24eff8a63599b54f711bbe3863ec727c47d

Original source revision from `.cargo_vcs_info.json`: 34f75c49f93de5e2ff943483cabc5b6d0c5df918, path noq-proto.

License: MIT OR Apache-2.0. Copyright notices and complete license texts are retained in LICENSE-MIT and LICENSE-APACHE. Backported upstream code and tests use those same repository licenses.

## Upstream changes

- https://github.com/n0-computer/noq/pull/866, merged commit 3ff2822f9e6ef327501f75cd6bc0f7d0f3e9b76a: adaptive coalescing with ceiling division, minimum retained chunk size, ordered lossless and rounding-boundary regressions.
- https://github.com/n0-computer/noq/pull/867, merged commit b70f1f2e444d3afdfb7a0cdf2b3d9f14c859a2ea: explicit compaction discard offset, preserving unread unordered bytes, and its regression.

These exact assembler changes were backported without unrelated main-branch changes. The published crates index listed 1.4.0 as the latest release when this patch was prepared on 2026-11-09.

## Local changes

Bluefin's local change adds `coalesce_contiguous` after adaptive compaction still exceeds MAX_CHUNKS. It first counts actual disjoint runs and rejects more than MAX_CHUNKS before allocating or copying. Otherwise it merges each adjacent run, retaining single chunks without copying. It operates only on the sorted, deduplicated output of defragment. Temporary payload allocation and bytes copied per call are at most the buffered unique payload, which the transport receive window already bounds. The original MAX_CHUNKS and COMPACT_THRESHOLD protections remain, with no receive/send-window reduction.

Local regressions cover mixed large/small contiguous data behind a missing byte, exact ordered and unordered bytes/offsets, overlapping retransmissions, copy bounds, no repeated copy of a single retained run, and rejection of real gap floods before copying. Existing assembler and protocol regressions remain.

A subsequent independent allocation regression reproduced retained-memory amplification in the upstream PR867 compaction path, independently of the local fallback. Slices of one compaction allocation survived across generations through unrelated output runs while the accounting reported only their lengths. Bluefin now transfers an independently owned buffer for each copied output run rather than splitting one shared backing allocation. Adaptive size thresholds, ordering, overlap trimming, chunk/run limits and receive/send windows are unchanged. Buffer capacity and allocator overhead remain estimates, not a hard memory quota.

The standalone `qa/compaction-retention` check includes this production assembler verbatim and measures live requested allocations with realistic frame overhead, an 8 MiB offset span, growing separated runs and agreeing duplicates. It checks exact output and allocation release after drop. BuildCore runs it before producing Apple libraries. It is test-only and is not a client dependency or artifact. The earlier four-window regression guard fails before the repair and is not a newly asserted protocol bound.

Local implementation and tests were authored by gpt-6.1-sol through T3 Code -> Codex harness. No third-party issue proposal code was copied. The local change still requires independent integration review; this implementation task does not claim to independently review itself.

The two client Cargo roots select this modified crate with explicit path patches. No registry source was edited. Remove the backport only after a compatible maintained release contains and qualifies all these fixes. No advisory is suppressed or remapped by this patch.
