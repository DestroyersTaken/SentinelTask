#!/usr/bin/env bash
# Build the distributable files into dist/:
#   SentinelTask-<ver>-portable.zip   unzip-and-run, settings stay beside the exe
#   SentinelTask-<ver>-setup.exe      per-user installer (no admin needed)
#   SentinelTask-<ver>-installer.zip  the installer, zipped (browsers often block .exe)
#   SHA256SUMS.txt
#
# Usage: packaging/make-dist.sh [path/to/SentinelTask.exe]
# Without an argument it cross-compiles with MinGW (x86_64-pc-windows-gnu).
# Needs: zip, makensis (apt install zip nsis).
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
EXE=${1:-}
if [[ -z "$EXE" ]]; then
  cargo build --release --target x86_64-pc-windows-gnu
  EXE=target/x86_64-pc-windows-gnu/release/SentinelTask.exe
fi

DIST=dist
STAGE=$DIST/stage
rm -rf "$DIST"
mkdir -p "$STAGE/SentinelTask"

# Portable zip.
cp "$EXE" "$STAGE/SentinelTask/SentinelTask.exe"
cp README.md "$STAGE/SentinelTask/README.md"
cp LICENSE "$STAGE/SentinelTask/LICENSE.txt"
cp packaging/portable.txt "$STAGE/SentinelTask/portable.txt"
(cd "$STAGE" && zip -q -9 -r "../SentinelTask-$VERSION-portable.zip" SentinelTask)

# Installer.
makensis -V2 -DVERSION="$VERSION" -DSRCDIR="$(realpath "$STAGE/SentinelTask")" \
  -DDOCDIR="$(pwd)" -DOUTFILE="$(realpath "$DIST")/SentinelTask-$VERSION-setup.exe" \
  packaging/installer.nsi

# The installer zipped too: many browsers and chat apps block a bare .exe.
(cd "$DIST" && zip -q -9 "SentinelTask-$VERSION-installer.zip" "SentinelTask-$VERSION-setup.exe")

rm -rf "$STAGE"
(cd "$DIST" && sha256sum ./*.zip ./*.exe > SHA256SUMS.txt)
ls -la "$DIST"
