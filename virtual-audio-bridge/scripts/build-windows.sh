#!/usr/bin/env bash
# Compila vab-engine.exe para Windows x64 (desde Linux con MinGW) y genera el
# instalador con NSIS. Requisitos: rustup target x86_64-pc-windows-gnu,
# gcc-mingw-w64-x86-64, g++-mingw-w64-x86-64, cmake, nasm, nsis.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/engine"
VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p "$ROOT/dist"
cp target/x86_64-pc-windows-gnu/release/vab-engine.exe "$ROOT/dist/"
x86_64-w64-mingw32-strip "$ROOT/dist/vab-engine.exe"
cd "$ROOT/installer"
makensis -V2 -DVERSION="$VERSION" vab.nsi
cd "$ROOT/dist"
sha256sum vab-engine.exe "VirtualAudioBridge-$VERSION-setup.exe" > SHA256SUMS.txt
cat SHA256SUMS.txt
