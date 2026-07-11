#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 0 ]]; then
  echo "usage: sccache-rustc-wrapper.sh <rustc> [args...]" >&2
  exit 2
fi

if [[ -n "${SCCACHE_BIN:-}" ]]; then
  sccache_bin="$SCCACHE_BIN"
elif sccache_bin="$(command -v sccache 2>/dev/null)" && [[ -n "$sccache_bin" ]]; then
  :
else
  exec "$@"
fi

manifest_dir="${CARGO_MANIFEST_DIR:-$PWD}"
candidate="$manifest_dir"
while [[ "$candidate" != "/" && -n "$candidate" ]]; do
  if [[ -e "$candidate/.git" ]]; then
    case ":${SCCACHE_BASEDIRS:-}:" in
      *":$candidate:"*) ;;
      *) export SCCACHE_BASEDIRS="$candidate${SCCACHE_BASEDIRS:+:$SCCACHE_BASEDIRS}" ;;
    esac
    break
  fi
  candidate="${candidate%/*}"
  [[ -n "$candidate" ]] || candidate="/"
done

if [[ "${GARYX_SCCACHE_PRINT_CONFIG:-0}" == "1" ]]; then
  echo "SCCACHE_BIN=$sccache_bin"
  echo "SCCACHE_BASEDIRS=${SCCACHE_BASEDIRS:-}"
  exit 0
fi

exec "$sccache_bin" "$@"
