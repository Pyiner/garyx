#!/usr/bin/env bash
# Ad-hoc sign the Garyx CLI with a stable macOS code-signing identifier.

set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Skipping macOS codesign on non-Darwin host."
  exit 0
fi

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 /path/to/garyx" >&2
  exit 2
fi

binary="$1"
identifier="${CODESIGN_IDENTIFIER:-com.garyx.gateway}"

if [[ ! -f "$binary" ]]; then
  echo "Error: binary does not exist: $binary" >&2
  exit 1
fi

/usr/bin/codesign --force --sign - --identifier "$identifier" "$binary"
/usr/bin/codesign --verify --verbose=2 "$binary"
