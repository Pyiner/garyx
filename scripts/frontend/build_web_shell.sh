#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
FRONTEND_DIR="$ROOT_DIR/desktop/garyx-desktop"
OUT_DIR="$FRONTEND_DIR/out/web"
TARGET_DIR="${GARYX_WEB_FRONTEND_DIR:-$OUT_DIR}"

if ! command -v npm >/dev/null 2>&1; then
  echo "npm is required to build web shell frontend" >&2
  exit 1
fi

cd "$FRONTEND_DIR"

if [[ ! -d node_modules ]]; then
  npm install
fi

npm run build:web

if [[ "$TARGET_DIR" != "$OUT_DIR" ]]; then
  mkdir -p "$TARGET_DIR"
  rsync -a --delete "$OUT_DIR/" "$TARGET_DIR/"
fi

echo "Built web shell bundle at: $OUT_DIR"
echo "Gateway web shell target: $TARGET_DIR"
