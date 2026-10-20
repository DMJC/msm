#!/usr/bin/env bash
# Build the .deb and .rpm packages into target/packages/.
# Needs: cargo install cargo-deb cargo-generate-rpm
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release
cargo deb --no-build
cargo generate-rpm
mkdir -p target/packages
cp target/debian/*.deb target/generate-rpm/*.rpm target/packages/
ls -l target/packages
