# Bluefin

Bluefin is this repository's Rust media-server fork of [Ferrofin](https://github.com/mangoleaf/ferrofin). It implements the Jellyfin HTTP API and retains the existing server, database adoption, media processing, and plugin implementation. Development and issues for Bluefin belong in [this repository](https://github.com/glopyglerky/bluefin).

The project identity is Bluefin. The executable is still `ferrofin-server`, the crates are still `ferrofin-*`, and configuration uses `FERROFIN_*`. These names are compatibility contracts for current consumers. See the [design and divergence record](docs/DIVERGENCE.md) before changing them.

## Current state

This is an existing Rust server implementation, not a newly completed port. Its workspace includes API handlers, SQLite persistence, media encoding, providers, Live TV, and WASM plugins. [Feature status](docs/FEATURES.md) describes the inherited implementation. The results below are inherited measurements of the named Ferrofin commits, not fresh Bluefin build or deployment qualification. In particular, the recorded parity gate did not pass.

Full native Rust conversion and 100% memory-safe Rust using Lustr/BlueFe are future goals. The current toolchain is pinned in [rust-toolchain.toml](rust-toolchain.toml). The workspace still uses native dependencies including SQLite and jemalloc, external ffmpeg/ffprobe, and a separately built Jellyfin web client. This repository does not establish whole-stack memory safety or current Lustr/BlueFe support.

<a id="quickstart"></a>

## Start from source

From a clone of this repository, with the pinned Rust toolchain and the required native build dependencies available:

```sh
cargo run --locked -p ferrofin-server -- --data-dir ./data --bind 127.0.0.1 --port 8096
```

This command starts a server and creates data. On a fresh database the inherited server seeds an `admin` account without a password. Keep it on loopback until setup is complete, or configure `FERROFIN_ADMIN_PASSWORD` before first start. A web setup also needs the Jellyfin web assets configured through `FERROFIN_WEB_DIR`; a native Jellyfin client can connect to the HTTP endpoint. ffmpeg/ffprobe provide transcoding. See [configuration](docs/CONFIG.md) and [installation reference](docs/INSTALL.md).

The installation reference and Helm chart retain upstream artifact locations. Images, packages, release badges, and benchmark claims published by Ferrofin do not establish a Bluefin release. Build and qualify the exact source and artifact you intend to use.

## Migrating from Jellyfin

Database adoption is one-way. Back up the entire data and configuration state, including database WAL/SHM companions, before attempting it. Never run two servers against the same database. Read the [migration procedure](docs/INSTALL.md#migrate-an-existing-jellyfin-installation), [adoption matrix](gates/adoption/README.md), and [upgrade notes](docs/UPGRADING.md). Rollback requires the original backup; it cannot recover new activity recorded after adoption.

## Source map

| Path | Role |
| --- | --- |
| [apps/ferrofin-server](apps/ferrofin-server) | Server executable, composition root, startup and configuration |
| [crates/ferrofin-api](crates/ferrofin-api) | Jellyfin-compatible HTTP routes |
| [crates/ferrofin-core](crates/ferrofin-core) | Core media-server behavior |
| [crates/ferrofin-db](crates/ferrofin-db) | Database access and migrations |
| [crates](crates) | Models, traits, providers, encoding, extensions and other workspace libraries |
| [contracts](contracts) | Plugin contracts |
| [contrib](contrib) and [charts/ferrofin](charts/ferrofin) | Packaging and deployment definitions with retained compatibility names |
| [gates](gates) and [Makefile](Makefile) | Verification, adoption, parity and benchmark tooling |
| [docs](docs) | Architecture, operation references and historical results |
| [vendor](vendor) | Retained vendored source; see the workspace patch configuration |

Workspace tests use `cargo test --locked`; formatting uses `cargo fmt --all --check`. The [CI workflow](.github/workflows/ci.yml) defines additional checks. Gates that launch servers, containers, or adoption tests require their documented fixtures and isolated state. A documentation change does not qualify those paths.

## Results

The generated block is preserved for the existing results publisher. Its Ferrofin names and source commits identify historical evidence. It must not be relabeled as a Bluefin benchmark.

<!-- BEGIN GENERATED RESULTS — do not edit by hand. Regenerate with: make publish-results (gates/publish.py) -->
### Speed and memory against Jellyfin

Ferrofin **v1.5.0** against **Jellyfin 12.4.0**: the median of 3 runs on one host (32 cores, 125 GiB), each server alone in its container, over the same library (3,001 movies, 250 series, 7,490 episodes, 50 HDR videos), 2026-11-08. One host and one library: other hardware and libraries give other numbers.

**13.0× faster cold start** · **2.6–5.2× the speed on 5 screens at 5 screens/s** · **2.4× lighter peak memory under load** · **2.1× lighter idle memory**

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/results/v1.5.0/screens-loaded-dark.svg">
  <img alt="Screens at 5 screens/s (loaded): p50 latency: Ferrofin against Jellyfin 12.4.0 on 5 comparable cells, as fast or faster on 5; from 2.6× faster to 5.2× faster." src="docs/results/v1.5.0/screens-loaded.svg">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/results/v1.5.0/startup-dark.svg">
  <img alt="Time to first screen: speed-up: Ferrofin against Jellyfin 12.4.0 on 3 comparable cells, as fast or faster on 3; from 2.0× faster to 13.0× faster." src="docs/results/v1.5.0/startup.svg">
</picture>

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/results/v1.5.0/memory-dark.svg">
  <img alt="Memory: Ferrofin against Jellyfin 12.4.0 on 2 comparable cells, as fast or faster on 2; from 2.1× lighter to 2.4× lighter." src="docs/results/v1.5.0/memory.svg">
</picture>

Not compared in this release, because the two servers did different work there or the evidence is incomplete: the search screen, the search endpoints, the image endpoints, CPU per screen. The search screen therefore has no comparison yet.

Measured from commit `961c0b412`. Every graph and table, the withheld cells and the sources: [docs/results/v1.5.0/README.md](docs/results/v1.5.0/README.md); every number with its range: [`docs/BENCHMARKS.md`](docs/BENCHMARKS.md).

### Adopting an existing Jellyfin database

Each supported Jellyfin version and upgrade route, adopted in place by Ferrofin **v1.5.0** (commit `961c0b412329`, 2026-11-08): **21 of 21 passed**. Adoption is one-way; see [Migrating from Jellyfin](#migrating-from-jellyfin). Every route: [docs/results/v1.5.0/README.md](docs/results/v1.5.0/README.md).

### Answering like Jellyfin

The parity gate replays the **2,353** requests jellyfin-web makes against Jellyfin 12.4.0 and Ferrofin **v1.5.0** (commit `961c0b412`, 2026-11-08) and compares the answers. Of the 2,307 it could compare, **2,198 are identical**, **1 hold the same answer in another order** (each order recorded in the ledger), and **107 are known differences**, each recorded in the parity ledger with the issue that tracks it, or the parity row that accepts it; **0 are filed as issues** and **1 is new**. 46 were not compared because Jellyfin itself failed them. **The parity gate did not pass** (new differences): 1 request differs in ways the ledger does not cover yet. Every API area: [docs/results/v1.5.0/README.md](docs/results/v1.5.0/README.md).

### Release over release

The same headline numbers for every release published so far (v1.4.0, v1.5.0), each against the Jellyfin release it was measured with. Only releases measured on the same host class and fixture are joined by a line.

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/results/history-dark.svg">
  <img alt="Release over release: 2 releases, v1.4.0 to v1.5.0, on 1 host and fixture series; the latest, v1.5.0 against Jellyfin 12.4.0: cold start 250 ms vs 3257 ms, screens at 5 screens/s 6 ms vs 31 ms, peak memory under load 245 MiB vs 598 MiB, idle memory 197 MiB vs 404 MiB." src="docs/results/history.svg">
</picture>

<!-- END GENERATED RESULTS -->

## Development and provenance

Read the [architecture](docs/ARCHITECTURE.md), [extension model](docs/EXTENSIONS.md), and [design and divergence record](docs/DIVERGENCE.md). Existing contributor and release documents describe inherited workflows; Bluefin changes should target this repository. The former upstream project pitch and personal release narrative have been replaced by this source map and current-state description; their originals remain in Git history.

Bluefin retains the [GPL license](LICENSE), source history, and upstream notices. It is independently maintained and is not affiliated with or endorsed by Jellyfin. Ferrofin and Jellyfin contributors supplied the implementation and compatibility work on which this fork builds.
