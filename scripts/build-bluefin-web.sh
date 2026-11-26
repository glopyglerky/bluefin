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
    --target wasm32-unknown-unknown --no-default-features --features ui --lib

scratch=$(mktemp -d "$(dirname -- "$output")/.bluefin-web.XXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
trap 'exit 1' HUP INT TERM
wasm-bindgen --target web --out-dir "$scratch" \
    "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/bluefin_universal.wasm"
cp "$app/index.html" "$app/media-worker.js" "$app/mesh-policy.json" "$root/LICENSE" "$scratch/"
cp -R "$app/vendor" "$scratch/vendor"
find "$scratch" -type d -exec chmod 755 {} +
find "$scratch" -type f -exec chmod 644 {} +
mv -T -n -- "$scratch" "$output"
if [ -d "$scratch" ]; then
    echo "Output directory was created during the build: $output" >&2
    exit 2
fi
trap - EXIT HUP INT TERM
