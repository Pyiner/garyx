#!/bin/sh
set -eu

if command -v garyx >/dev/null 2>&1; then
  exec garyx debug "$@"
fi

SEARCH_DIR=$(pwd)
while [ "$SEARCH_DIR" != "/" ]; do
  MANIFEST_PATH="$SEARCH_DIR/Cargo.toml"
  if [ -f "$MANIFEST_PATH" ]; then
    exec cargo run -q -p garyx --bin garyx --manifest-path "$MANIFEST_PATH" -- debug "$@"
  fi
  SEARCH_DIR=$(dirname "$SEARCH_DIR")
done

echo "garyx CLI not found and no repo-local Cargo.toml found from current directory" >&2
exit 1
