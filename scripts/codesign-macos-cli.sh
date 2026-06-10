#!/usr/bin/env bash
# Sign Garyx CLI binaries with a stable macOS code-signing identity.
#
# macOS TCC stores permission grants against the binary's designated
# requirement. A certificate-backed signature keeps that requirement stable
# across rebuilds ("identifier + certificate anchor"), so grants such as
# Downloads-folder access survive reinstalling the gateway. Ad-hoc
# signatures hash the binary itself (CDHash), which changes on every build
# and resets TCC permission, so ad-hoc is only the fallback for hosts
# without a signing certificate (such as CI release runners).

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

find_identity() {
  local pattern="$1"

  /usr/bin/security find-identity -v -p codesigning 2>/dev/null |
    awk -v pattern="$pattern" '$0 ~ pattern { print $2; exit }'
}

resolve_identity() {
  if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
    printf '%s\n' "$CODESIGN_IDENTITY"
    return 0
  fi

  local identity

  identity="$(find_identity "Developer ID Application")"
  if [[ -n "$identity" ]]; then
    printf '%s\n' "$identity"
    return 0
  fi

  identity="$(find_identity "Apple Development")"
  if [[ -n "$identity" ]]; then
    printf '%s\n' "$identity"
    return 0
  fi

  printf -- '-\n'
}

identity="$(resolve_identity)"

sign_args=(--force --sign "$identity" --identifier "$identifier")
if [[ "$identity" == "-" ]]; then
  echo "Warning: no code-signing certificate found; falling back to ad-hoc" \
    "signing. macOS TCC permission grants will not survive rebuilds." >&2
else
  # Local signatures do not need a trusted timestamp; skipping it avoids a
  # network round-trip to Apple's timestamp server on every build.
  sign_args+=(--timestamp=none)
fi

for binary in "$@"; do
  if [[ ! -f "$binary" ]]; then
    echo "Error: binary does not exist: $binary" >&2
    exit 1
  fi

  /usr/bin/codesign "${sign_args[@]}" "$binary"
  /usr/bin/codesign --verify --verbose=2 "$binary"

  # No early exit in awk: quitting mid-stream can SIGPIPE codesign, which
  # pipefail turns into a spurious 141 script failure.
  actual_identifier="$(
    /usr/bin/codesign -dv --verbose=4 "$binary" 2>&1 |
      awk -F= '/^Identifier=/ && !found { print $2; found = 1 }'
  )"
  if [[ "$actual_identifier" != "$identifier" ]]; then
    echo "Error: codesign identifier mismatch for $binary: expected $identifier, got $actual_identifier" >&2
    exit 1
  fi
done
