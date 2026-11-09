# Bluefin design and divergence record

Maintain this file when ownership, layout, compatibility names, toolchain support, or artifact publication changes. Record the exact source references and the checks that justify each change.

## Provenance and scope

GitHub's repository API verified the fork parent and source as [mangoleaf/ferrofin](https://github.com/mangoleaf/ferrofin). The inspected Bluefin `main` baseline was `b7d124a22d49bacf19f6c3c940333f508d647fb6`. This is a provenance observation, not a claim that every file matches the current upstream head.

Bluefin owns its project presentation and future changes. This adjustment replaces the upstream primary README, points Cargo repository and package homepage metadata at Bluefin, identifies the retained server package as Bluefin in descriptive metadata, and labels the installation guide as an upstream artifact reference. It changes no executable behavior, source location, version, license, dependency, lockfile, or deployment.

## Layout and naming decisions

| Retained implementation or name | Exact contract and reason | Requirement before changing |
| --- | --- | --- |
| `apps/ferrofin-server`, `crates/ferrofin-*` | Root `Cargo.toml` workspace members and path dependencies, server manifest library/bin targets, and Rust imports use these paths and crate identities. These are the actual Bluefin sources. | Audit imports, manifests, consumers and build references; qualify the renamed workspace. |
| `ferrofin-server`, `ferrofin.service`, service user and filesystem names | `Dockerfile` builds and launches that binary. `contrib/systemd/ferrofin.service` and `apps/ferrofin-server/Cargo.toml` package contents use the existing executable, account and configuration paths. Renaming would change installed-state contracts. | Qualify packaging, upgrades, state continuity and rollback with existing consumers. |
| `FERROFIN_*` and Jellyfin wire/database names | Configuration, container environment and service definitions use the existing prefix. Jellyfin client and database compatibility is the server's purpose. | Preserve aliases and migration behavior, then test configuration, clients and database adoption. |
| `charts/ferrofin`, inherited registry/package references | Chart consumers and installation/release documents refer to existing upstream artifacts. A Bluefin artifact was not built or verified in this change. Inventing a new registry URL would misdirect installs. | Publish and qualify an owned artifact before changing installation destinations. |
| Root README generated results block | `gates/publish.py` requires its markers; `.github/workflows/ci.yml` checks the rendered block against committed summaries. The named Ferrofin commits and results are historical evidence. | Keep the renderer contract; only publish new performance claims from new measurements. |
| `vendor/icu_collator` and native dependencies | The root manifest excludes the vendored crate from workspace membership and retains patch configuration. SQLite, jemalloc and media tools remain implementation dependencies. | Audit feature/target selection, native bindings and behavior before substitution. |

No physical source move was made. The existing workspace already separates the executable, libraries, contracts and qualification tooling. A cosmetic directory move would require build qualification and consumer changes without improving that separation.

## Toolchain and memory safety

`rust-toolchain.toml` pins Rust 1.99.1; the Dockerfile and CI refer to that toolchain. This source inspection does not certify toolchain availability or a successful current build. Rust application code does not prove that its allocator, SQLite bindings, crypto dependencies, web client, child processes, or complete artifact are native or memory safe.

Full native Rust conversion and 100% memory-safe Rust with Lustr/BlueFe are future goals. They require separate dependency and unsafe-code audits, compiler support, meaningful parity and failure-path checks, and working builds. No compiler migration or safety certification is included here.

## Verification boundary

This ownership adjustment uses bounded documentation-link and manifest comparisons, preserves the generated results block, and checks its publisher without building the product. Historical results and upstream feature documents remain attributed to their source revisions. Runtime acceptance, new platform support and deployment are separate work.
