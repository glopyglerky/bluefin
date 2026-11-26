#!/bin/sh
set -eu

if [ "$#" -ne 1 ] || [ -e "$1" ] || [ -L "$1" ]; then
    echo "Usage: sh scripts/build-bluefin-web.sh <new output directory>" >&2
    exit 2
fi
case "$1" in
    /*) output=$1 ;;
    *) output=$PWD/$1 ;;
esac
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
app="$root/apps/bluefin-universal"
: "${CARGO_TARGET_DIR:?Set CARGO_TARGET_DIR to a build directory outside the checkout}"
case "$CARGO_TARGET_DIR" in
    /*) ;;
    *) CARGO_TARGET_DIR=$PWD/$CARGO_TARGET_DIR ;;
esac
export CARGO_TARGET_DIR

if [ "$(wasm-bindgen --version)" != "wasm-bindgen 0.2.126" ]; then
    echo "Install wasm-bindgen CLI 0.2.126 to match the browser crate lockfile." >&2
    exit 2
fi
cargo build --manifest-path "$app/Cargo.toml" --locked --release \
    --target wasm32-unknown-unknown --no-default-features --lib

scratch=$(mktemp -d "$(dirname -- "$output")/.bluefin-web.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
wasm-bindgen --target web --out-dir "$scratch/engine-unversioned" \
    "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/bluefin_universal.wasm"
node "$root/scripts/package-bluefin-web.cjs" "$scratch/engine-unversioned" "$scratch/bundle" "$root"
mv -T -n -- "$scratch/bundle" "$output"
if [ -d "$scratch/bundle" ]; then
    echo "Output directory was created during the build: $output" >&2
    exit 2
fi
rm -rf -- "$scratch"
trap - EXIT HUP INT TERM
