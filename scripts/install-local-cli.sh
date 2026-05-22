#!/usr/bin/env bash
# Build and install the local Garyx CLI, preserving the macOS TCC identity.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_DIR="$HOME/.cargo/bin"
DESTINATION="${INSTALL_DIR}/garyx"

cd "$REPO_ROOT"

cargo build --release -p garyx
bash scripts/build-cctty-sidecar.sh
mkdir -p "$INSTALL_DIR"
install -m 755 target/release/garyx "$DESTINATION"
install -m 755 target/release/cctty "${INSTALL_DIR}/cctty"
bash scripts/codesign-macos-cli.sh "$DESTINATION"
CODESIGN_IDENTIFIER="com.garyx.cctty" bash scripts/codesign-macos-cli.sh "${INSTALL_DIR}/cctty"
"$DESTINATION" --version
"${INSTALL_DIR}/cctty" --version
