# Native client dependency boundary

Run this check from the repository root, with `CARGO_TARGET_DIR` set to bounded
build storage outside source:

```sh
cargo run --locked --manifest-path gates/client-boundary/Cargo.toml -- .
```

Append `android`, `universal` or `all` to select additional clients. The default
is `apple`, so existing Apple build invocations retain their behavior.

The check resolves each selected client's locked Cargo graph with all features. It
rejects any local crate outside the explicit client/transport/wire-contract
allowlist, including renamed server crates. It also rejects known direct Seerr
service markers in native source. Generic Bluefin discovery protocol calls are
allowed. The tool itself is development tooling and is not a client dependency.
`BuildCore.rs` runs it before producing Apple Rust libraries, and CI runs it.

Apple, Android JNI/Kotlin and Universal Rust/browser sources are registered.
Universal checks include the embedded web player, media worker and vendored HLS
adapter. The removed external browser companion is no longer an input.
Run checks on a stable source candidate; other clients may still be under active
development. This guard does not establish installed platform acceptance. Review
source and final packaged resources as well. A marker scan cannot prove that
arbitrary code never makes a forbidden network call.

Keep provider adapters and web administration bundles outside these allowlisted
paths. Shared wire contracts must contain only generic Bluefin types, with no
provider-specific configuration, credentials, account mapping or request logic.
