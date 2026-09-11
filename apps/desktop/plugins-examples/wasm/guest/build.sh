#!/usr/bin/env bash
# Build the iTunes WASM guest for Bilusic.
# Requires: rustup target add wasm32-unknown-unknown
set -euo pipefail
cd "$(dirname "$0")"
cargo build --target wasm32-unknown-unknown
echo "wasm produced at: target/wasm32-unknown-unknown/debug/guest.wasm"
