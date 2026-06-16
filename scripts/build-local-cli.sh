#!/usr/bin/env bash
# Build the local Garyx CLI and sign the release binary with the stable TCC identity.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_BINARY="${REPO_ROOT}/target/release/garyx"

cd "$REPO_ROOT"

# Garyx no longer bundles a Bun runtime; workflows resolve `bun` from PATH and
# error with install instructions when it is missing.
cargo build --release -p garyx
bash scripts/codesign-macos-cli.sh "$TARGET_BINARY"
"$TARGET_BINARY" --version
