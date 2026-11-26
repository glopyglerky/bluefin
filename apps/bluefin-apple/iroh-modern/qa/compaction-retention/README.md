# Retained compaction allocation regression

Run this standalone check with stable Rust and its committed lockfile:

```sh
cargo +stable run --locked --manifest-path qa/compaction-retention/Cargo.toml
```

Set `CARGO_TARGET_DIR` to bounded storage outside source. The check includes the
selected vendored assembler verbatim with the current consumer's bytes/tinyvec
versions. It counts live requested Rust allocations in an isolated single-threaded
process, rather than asserting the assembler's own estimates or measuring RSS.

The input has one withheld byte, a 3.5 MiB prefix and 512 separated islands.
Growing islands plus agreeing duplicate frames trigger repeated compaction. All
offsets remain in the 8 MiB stream window. The check guards against retaining
more than four entire windows, then checks exact unordered output and that drop
releases every measured allocation. That generous envelope is a regression guard,
not a new protocol memory quota. Application-owned returned chunks are not retained.

This catches old compaction allocations pinned by slices across unrelated output
runs. Keep the ordinary protocol byte/overlap/gap tests as separate checks. This
test-only allocator and binary are never linked into client artifacts.
