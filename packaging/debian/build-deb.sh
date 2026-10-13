#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

ASSETS_DIR="target/debian-assets"
mkdir -p "$ASSETS_DIR/icons" "$ASSETS_DIR/man" "$ASSETS_DIR/completions"

for size in 48 64 128 256 512; do
  rsvg-convert -w "$size" -h "$size" assets/logo.svg -o "$ASSETS_DIR/icons/${size}.png"
done

if [ -x "./target/release/examples/man_pages" ]; then
  ./target/release/examples/man_pages "$ASSETS_DIR/man"
elif [ -x "./target/release/man_pages" ]; then
  ./target/release/man_pages "$ASSETS_DIR/man"
else
  cargo run --release -p flowshot-cli --example man_pages "$ASSETS_DIR/man"
fi

./target/release/flowshot completions bash > "$ASSETS_DIR/completions/flowshot.bash"
./target/release/flowshot completions zsh > "$ASSETS_DIR/completions/_flowshot"
./target/release/flowshot completions fish > "$ASSETS_DIR/completions/flowshot.fish"

cargo deb -p flowshot-cli --no-build
