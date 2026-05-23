#!/usr/bin/env bash
# Build the cctty sidecar used by the Claude Agent SDK provider.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${TARGET:-${1:-}}"
OUT_DIR="${OUT_DIR:-${REPO_ROOT}/target/release}"
CCTTY_REF="${CCTTY_REF:-ef60376002b44e19e35087e06c033fc78b452c33}"
CCTTY_GIT="${CCTTY_GIT:-https://github.com/Pyiner/cctty.git}"
CCTTY_SOURCE_DIR="${CCTTY_SOURCE_DIR:-}"
MANAGED_CCTTY_SOURCE=0

if [[ -z "$CCTTY_SOURCE_DIR" && -f "${REPO_ROOT}/../cctty/Cargo.toml" ]]; then
  CCTTY_SOURCE_DIR="${REPO_ROOT}/../cctty"
fi

if [[ -z "$CCTTY_SOURCE_DIR" ]]; then
  CCTTY_SOURCE_DIR="${REPO_ROOT}/target/cctty-src"
  MANAGED_CCTTY_SOURCE=1
  if [[ ! -d "$CCTTY_SOURCE_DIR/.git" ]]; then
    rm -rf "$CCTTY_SOURCE_DIR"
    git clone "$CCTTY_GIT" "$CCTTY_SOURCE_DIR"
  fi
  git -C "$CCTTY_SOURCE_DIR" fetch --tags "$CCTTY_GIT" "$CCTTY_REF"
  git -C "$CCTTY_SOURCE_DIR" checkout --detach "$CCTTY_REF"
fi

if [[ "$MANAGED_CCTTY_SOURCE" == "1" ]] && ! grep -q '^\[workspace\]' "$CCTTY_SOURCE_DIR/Cargo.toml"; then
  printf '\n[workspace]\n' >> "$CCTTY_SOURCE_DIR/Cargo.toml"
fi

BUILD_TARGET_DIR="${REPO_ROOT}/target/cctty-sidecar"
if [[ -n "$TARGET" ]]; then
  if [[ "$TARGET" == *linux* ]] && command -v cargo-zigbuild >/dev/null 2>&1; then
    cargo zigbuild --release --manifest-path "$CCTTY_SOURCE_DIR/Cargo.toml" \
      --target "${TARGET}.2.17" --target-dir "$BUILD_TARGET_DIR"
  else
    cargo build --release --manifest-path "$CCTTY_SOURCE_DIR/Cargo.toml" \
      --target "$TARGET" --target-dir "$BUILD_TARGET_DIR"
  fi
  BUILT_BIN="${BUILD_TARGET_DIR}/${TARGET}/release/cctty"
else
  cargo build --release --manifest-path "$CCTTY_SOURCE_DIR/Cargo.toml" \
    --target-dir "$BUILD_TARGET_DIR"
  BUILT_BIN="${BUILD_TARGET_DIR}/release/cctty"
fi

if [[ ! -f "$BUILT_BIN" ]]; then
  echo "Error: cctty build did not produce $BUILT_BIN" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
install -m 755 "$BUILT_BIN" "$OUT_DIR/cctty"

if [[ "$(uname -s)" == "Darwin" ]]; then
  CODESIGN_IDENTIFIER="com.garyx.cctty" \
    bash "$REPO_ROOT/scripts/codesign-macos-cli.sh" "$OUT_DIR/cctty"
fi

echo "$OUT_DIR/cctty"
