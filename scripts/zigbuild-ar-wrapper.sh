#!/usr/bin/env bash
# AR wrapper that works around two separate bugs:
#
# 1. cc-rs invokes `ar cq <archive> <objs...>`. GNU binutils ar and BSD
#    ar create the archive on-the-fly; llvm-ar wants `rcs`. We translate.
#
# 2. **zig 0.16.0's bundled llvm-ar is broken**: every operation that
#    would create a new archive errors out with "unable to open … No such
#    file or directory", even `rcs`. Empirically reproduces on macOS
#    arm64 / zig 0.16.0 against any non-Mach-O object (libsqlite3-sys's
#    sqlite3.o, ring's curve25519.o, …). Standalone homebrew `llvm@21`
#    llvm-ar handles the exact same invocation correctly, so we prefer
#    that when available and only fall back to `zig ar` as a last resort.
#
# Used by scripts/build-linux-release.sh via
#   AR_x86_64_unknown_linux_gnu=<this_script>
# and its aarch64 counterpart.

set -euo pipefail

pick_ar() {
  if [[ -n "${GARYX_AR:-}" && -x "${GARYX_AR}" ]]; then
    echo "$GARYX_AR"; return
  fi
  for candidate in \
      /opt/homebrew/opt/llvm@21/bin/llvm-ar \
      /opt/homebrew/opt/llvm/bin/llvm-ar \
      /usr/local/opt/llvm@21/bin/llvm-ar \
      /usr/local/opt/llvm/bin/llvm-ar; do
    if [[ -x "$candidate" ]]; then
      echo "$candidate"; return
    fi
  done
  if command -v llvm-ar >/dev/null 2>&1; then
    command -v llvm-ar; return
  fi
  # Last resort: zig's bundled llvm-ar (may be broken on 0.16.0).
  echo "__zig_ar__"
}

AR_BIN="$(pick_ar)"
run_ar() {
  if [[ "$AR_BIN" == "__zig_ar__" ]]; then
    exec zig ar "$@"
  fi
  exec "$AR_BIN" "$@"
}

if [[ $# -eq 0 ]]; then
  run_ar
fi

op="$1"; shift
case "$op" in
  cq|cqS|qc|qcs|cqs)
    run_ar rcs "$@"
    ;;
  *)
    run_ar "$op" "$@"
    ;;
esac
