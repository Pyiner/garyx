#!/usr/bin/env bash
# Prepare the compressed Bun runtime blob embedded into release Garyx binaries.

set -euo pipefail

TARGET="${1:-host}"
DESTINATION="${2:-}"

die() {
  echo "Error: $*" >&2
  exit 1
}

if [[ -z "$DESTINATION" ]]; then
  die "Usage: $0 <host|rust-target> /path/to/garyx-bun.xz"
fi

for command in xz mktemp; do
  command -v "$command" >/dev/null 2>&1 || die "Required command '${command}' not found."
done

TMPDIR="$(mktemp -d)"
trap 'rm -rf "${TMPDIR:-}"' EXIT

bash "$(dirname "$0")/download-bun-runtime.sh" "$TARGET" "$TMPDIR/garyx-bun"
mkdir -p "$(dirname "$DESTINATION")"
xz -9e -c "$TMPDIR/garyx-bun" > "$DESTINATION"

raw_size="$(wc -c < "$TMPDIR/garyx-bun" | tr -d ' ')"
compressed_size="$(wc -c < "$DESTINATION" | tr -d ' ')"
echo "Prepared embedded workflow runtime: $DESTINATION (${compressed_size} bytes from ${raw_size} bytes)"
