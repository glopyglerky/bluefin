# SQLx SQLite compatibility patch

This is the released `sqlx-sqlite` 0.9.0 crate, with only the upstream
`libsqlite3-sys` upper-bound changes applied to both published manifests.
All driver Rust source is unchanged. The package version remains 0.9.0.

- Registry origin: https://static.crates.io/crates/sqlx-sqlite/sqlx-sqlite-0.9.0.crate
- Original archive SHA256: `488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`
- Published VCS revision: `003b698e99e024f3621b8043a2426fde5b741171`, directory `sqlx-sqlite`.
- Upstream change allowing 0.38.x: https://github.com/transact-rs/sqlx/commit/94aafe3a68884d923b0798a767c8d7f6cfda89d2
- Upstream change allowing 0.39.x: https://github.com/transact-rs/sqlx/commit/10462fa4414c6c7f1ed1d39bb31678e3b726878c
- Combined manifest delta: `>=0.30.1, <0.38.0` to `>=0.30.1, <0.40.0`.
- Licenses: MIT OR Apache-2.0. The package's one-line license forwarding files
  are expanded here with the complete upstream license texts from the published
  VCS revision above, including the SQLx Contributors and LaunchBadge notices.
  This packaging correction changes no driver Rust source.
- The upstream package's standalone Cargo.lock is omitted; the workspace lock is authoritative.

The workspace selects released `libsqlite3-sys =0.39.1` (archive SHA256
`14a5f01b87b92705d07b433c06f96cbc764637d81a0bffb5734ce17cf0f9a2b3`),
which bundles SQLite 3.54.1. SQLx's existing `sqlite` feature continues to
select the bundled C library; no system-SQLite or SQLCipher substitution is used.
Rust 1.99.1 exceeds libsqlite3-sys's declared Rust 1.88 minimum.

The security floor is covered by `ferrofin-db/tests/sqlite_runtime.rs`, through
the actual SQLx connection and linked C API. It also checks synthetic WAL,
reopen, migrations, integrity and UUID/date storage behavior. Existing queue,
account, source-authority and server composition tests remain required.

Remove this local compatibility patch once a maintained released SQLx driver
supports the patched library, retaining the runtime security-floor test. This
does not establish installed-stack, native-client or release acceptance.
