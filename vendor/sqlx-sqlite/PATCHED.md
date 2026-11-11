# Vendored sqlx-sqlite 0.9.0

This is the crates.io `sqlx-sqlite` 0.9.0 package with one dependency-bound
change. Its `src/`, normalized `Cargo.toml`, original `Cargo.toml.orig`, and
both upstream license files are retained. The root `Cargo.toml` selects it
through `[patch.crates-io]`.

## Source and change

The published package has SHA-256
`488e99c397a62007e4229aec669a179816339afc6d2620ca6fa420dbee2e982c`.
Its source commit is `003b698e99e024f3621b8043a2426fde5b741171` in
<https://github.com/transact-rs/sqlx>.

The normalized manifest permits `libsqlite3-sys >=0.30.1, <0.40.0`
instead of `>=0.30.1, <0.38.0`. This is the bound accepted upstream in
[commit 10462fa4414c6c7f1ed1d39bb31678e3b726878c](https://github.com/transact-rs/sqlx/commit/10462fa4414c6c7f1ed1d39bb31678e3b726878c).
Every Rust source file and both licenses are unchanged from the published
package. `Cargo.toml.orig` records the original package manifest.

The workspace requires `libsqlite3-sys` 0.39.1, which bundles SQLite 3.54.1.
The published driver's older ceiling prevents that version from resolving.
The update fixes [RUSTSEC-2026-0459](https://rustsec.org/advisories/RUSTSEC-2026-0459.html)
while retaining one linked SQLite instance for SQLx and the direct C API.
The workspace continues to select bundled SQLite, not SQLCipher.

Remove this patch and vendor directory when a published SQLx driver admits
the repaired `libsqlite3-sys` version. Keep the workspace's minimum fixed
version and run the database tests and dependency security gate after removal.
