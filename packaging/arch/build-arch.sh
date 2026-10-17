#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

ARCH_VER="${1:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"

STAGE_DIR="target/arch-stage"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"

if [ -d "dist-binaries" ]; then
  find dist-binaries -type f -name flowshot -exec cp -v {} "$STAGE_DIR/flowshot" \;
  find dist-binaries -type f -name flowshot-daemon -exec cp -v {} "$STAGE_DIR/flowshot-daemon" \;
  find dist-binaries -type f -name man_pages -exec cp -v {} "$STAGE_DIR/man_pages" \;
elif [ -d "target/release" ]; then
  cp -v target/release/flowshot "$STAGE_DIR/flowshot"
  cp -v target/release/flowshot-daemon "$STAGE_DIR/flowshot-daemon"
  if [ -f target/release/examples/man_pages ]; then
    cp -v target/release/examples/man_pages "$STAGE_DIR/man_pages"
  else
    cp -v target/release/man_pages "$STAGE_DIR/man_pages"
  fi
else
  echo "No prebuilt binaries found in dist-binaries/ or target/release/" >&2
  exit 1
fi

chmod +x "$STAGE_DIR"/flowshot "$STAGE_DIR"/flowshot-daemon "$STAGE_DIR"/man_pages

cp packaging/arch/PKGBUILD.template "$STAGE_DIR/PKGBUILD"
sed -i "s/@PKGVER@/$ARCH_VER/g" "$STAGE_DIR/PKGBUILD"

cp packaging/flowshot.desktop.in "$STAGE_DIR/"
cp assets/logo.svg "$STAGE_DIR/"
cp LICENSE NOTICES README.md "$STAGE_DIR/"

(
  cd "$STAGE_DIR"
  makepkg -f --nodeps
  mv "flowshot-$ARCH_VER-1-x86_64.pkg.tar.zst" "flowshot-$ARCH_VER-1-x86_64-Arch.pkg.tar.zst"
  namcap PKGBUILD
  namcap "flowshot-$ARCH_VER-1-x86_64-Arch.pkg.tar.zst"
)

mkdir -p packaging/arch
cp "$STAGE_DIR/flowshot-$ARCH_VER-1-x86_64-Arch.pkg.tar.zst" packaging/arch/
echo "Arch package generated at packaging/arch/flowshot-$ARCH_VER-1-x86_64-Arch.pkg.tar.zst"
