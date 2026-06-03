#!/usr/bin/env bash
# Ad-hoc sign Garyx CLI binaries with a stable macOS code-signing identifier.

set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 /path/to/garyx [/path/to/another-garyx ...]" >&2
  exit 2
fi

identifier="${CODESIGN_IDENTIFIER:-com.garyx.gateway}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Skipping macOS codesign on non-Darwin host."
  exit 0
fi

for binary in "$@"; do
  if [[ ! -f "$binary" ]]; then
    echo "Error: binary does not exist: $binary" >&2
    exit 1
  fi

  /usr/bin/codesign --force --sign - --identifier "$identifier" "$binary"
  /usr/bin/codesign --verify --verbose=2 "$binary"

  actual_identifier="$(
    /usr/bin/codesign -dv --verbose=4 "$binary" 2>&1 |
      awk -F= '/^Identifier=/ { print $2; exit }'
  )"
  if [[ "$actual_identifier" != "$identifier" ]]; then
    echo "Error: codesign identifier mismatch for $binary: expected $identifier, got $actual_identifier" >&2
    exit 1
  fi
done
