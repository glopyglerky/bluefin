# Native client dependency boundary

Run this check from the repository root, with `CARGO_TARGET_DIR` set to bounded
build storage outside source:

```sh
cargo run --locked --manifest-path gates/client-boundary/Cargo.toml -- .
```

The check resolves the Apple client's locked Cargo graph with all features. It
rejects any local crate outside the explicit client/transport/wire-contract
allowlist, including renamed server crates. It also rejects known direct Seerr
service markers in native source. Generic Bluefin discovery protocol calls are
allowed. The tool itself is development tooling and is not a client dependency.
`BuildCore.rs` runs it before producing Apple Rust libraries, and CI runs it.

Register each additional native client and its exact dependency graph here when
its build is introduced. The current check covers the implemented Apple clients;
it does not claim to qualify platform packages that do not exist yet. Review
source and final packaged resources as well. A marker scan cannot prove that
arbitrary code never makes a forbidden network call.

Keep provider adapters and web administration bundles outside these allowlisted
paths. Shared wire contracts must contain only generic Bluefin types, with no
provider-specific configuration, credentials, account mapping or request logic.
