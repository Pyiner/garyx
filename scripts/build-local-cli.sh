#!/usr/bin/env bash
# Build the local Garyx CLI and sign the release binary with the stable TCC identity.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_BINARY="${REPO_ROOT}/target/release/garyx"

cd "$REPO_ROOT"

runtime_xz="${REPO_ROOT}/target/embedded-runtimes/host/garyx-bun.xz"
bash scripts/prepare-embedded-bun-runtime.sh host "$runtime_xz"
GARYX_EMBED_WORKFLOW_BUN_XZ="$runtime_xz" cargo build --release -p garyx
bash scripts/codesign-macos-cli.sh "$TARGET_BINARY"
"$TARGET_BINARY" --version
