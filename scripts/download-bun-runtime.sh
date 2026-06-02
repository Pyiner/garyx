#!/usr/bin/env bash
# Download the pinned Bun binary used by Garyx-managed workflow execution.

set -euo pipefail

BUN_VERSION="${BUN_VERSION:-1.3.14}"
TARGET="${1:-host}"
DESTINATION="${2:-}"

die() {
  echo "Error: $*" >&2
  exit 1
}

detect_host_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os" in
    Darwin)
      case "$arch" in
        arm64|aarch64) echo "aarch64-apple-darwin" ;;
        x86_64)        echo "x86_64-apple-darwin" ;;
        *) die "Unsupported macOS architecture: ${arch}" ;;
      esac
      ;;
    Linux)
      case "$arch" in
        x86_64|amd64)  echo "x86_64-unknown-linux-gnu" ;;
        aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
        *) die "Unsupported Linux architecture: ${arch}" ;;
      esac
      ;;
    *) die "Unsupported OS: ${os}" ;;
  esac
}

asset_for_target() {
  case "$1" in
    aarch64-apple-darwin)    echo "bun-darwin-aarch64.zip" ;;
    x86_64-apple-darwin)     echo "bun-darwin-x64-baseline.zip" ;;
    aarch64-unknown-linux-gnu) echo "bun-linux-aarch64.zip" ;;
    x86_64-unknown-linux-gnu)  echo "bun-linux-x64-baseline.zip" ;;
    *) die "Unsupported Bun runtime target: $1" ;;
  esac
}

if [[ "$TARGET" == "host" ]]; then
  TARGET="$(detect_host_target)"
fi

if [[ -z "$DESTINATION" ]]; then
  die "Usage: $0 <host|rust-target> /path/to/garyx-bun"
fi

for command in curl python3 mktemp; do
  command -v "$command" >/dev/null 2>&1 || die "Required command '${command}' not found."
done

ASSET="$(asset_for_target "$TARGET")"
URL="https://github.com/oven-sh/bun/releases/download/bun-v${BUN_VERSION}/${ASSET}"
TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR:-}"' EXIT

echo "Downloading Bun ${BUN_VERSION} for ${TARGET}..."
curl -fsSL "$URL" -o "$TMPDIR/$ASSET"

python3 - "$TMPDIR/$ASSET" "$TMPDIR/extract" <<'PY'
import pathlib
import sys
import zipfile

archive = pathlib.Path(sys.argv[1])
dest = pathlib.Path(sys.argv[2])
dest.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(archive) as zip_file:
    zip_file.extractall(dest)
PY

BUN_BIN="$(find "$TMPDIR/extract" -type f -name bun | head -n 1)"
if [[ -z "$BUN_BIN" ]]; then
  die "Downloaded archive did not contain a Bun binary."
fi

mkdir -p "$(dirname "$DESTINATION")"
install -m 755 "$BUN_BIN" "$DESTINATION"
echo "Installed workflow runtime binary: $DESTINATION"
