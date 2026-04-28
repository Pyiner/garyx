#!/usr/bin/env bash
# Build a Linux release binary natively from macOS (or any host) via
# `cargo-zigbuild`, which uses `zig cc` as the linker to produce binaries
# with a **pinned glibc floor** — no Docker/QEMU, no cross-compile GCC
# toolchain fuss, no surprise glibc 2.39 footgun when CI upgrades its
# runner image.
#
# Why this script exists
# ----------------------
# The GitHub Actions release workflow runs on ubuntu-22.04 (glibc 2.35).
# That's fine for most Linux distros released after ~2023, but many
# enterprise hosts (Debian 12, older RHEL derivatives) still ship glibc
# 2.31 or 2.34, so naive CI binaries fail on load with:
#
#     /lib64/libc.so.6: version `GLIBC_2.35' not found
#
# Pinning a floor via `cargo-zigbuild`'s target-suffix syntax
# (`x86_64-unknown-linux-gnu.2.17`) tells zig's libc to emit stub symbols
# from an older glibc ABI. 2.17 covers RHEL 7 / CentOS 7 and is the
# conservative default. Go lower only if you hit pre-2012 distros.
#
# Usage
# -----
#   scripts/build-linux-release.sh                # x86_64 + aarch64, glibc 2.17
#   ARCHS=x86_64 scripts/build-linux-release.sh   # x86_64 only
#   GLIBC=2.31 scripts/build-linux-release.sh     # raise floor (smaller binary)
#
# Prereqs
# -------
#   brew install zig
#   cargo install cargo-zigbuild --locked
#   rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu
#
# Output
# ------
#   dist/garyx-<version>-<target>/garyx
#   dist/garyx-<version>-<target>.tar.gz
#   dist/garyx-<version>-<target>.tar.gz.sha256

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

ARCHS="${ARCHS:-x86_64 aarch64}"
GLIBC="${GLIBC:-2.17}"

# Pull version from the workspace Cargo.toml (single source of truth).
# The line we want is `version = "X.Y.Z"` inside [workspace.package]; skip
# `version.workspace = true` and `version = { workspace = true }` forms.
VERSION="$(awk -F'"' '/^version *= *"/ {print $2; exit}' Cargo.toml)"
if [[ -z "$VERSION" ]]; then
  echo "Error: could not determine version from Cargo.toml" >&2
  exit 1
fi

echo "==> garyx ${VERSION} · glibc floor ${GLIBC} · archs: ${ARCHS}"

# Override AR for cross targets with our wrapper — cargo-zigbuild's default
# ar wrapper hits an llvm-ar bug when cc-rs calls `ar cq` to create
# archives for C-heavy crates (libsqlite3-sys ships the whole sqlite
# amalgamation, so every build tickles this). The wrapper normalizes
# `cq` → `rcs` before forwarding to `zig ar`. See the wrapper for
# upstream bug context.
AR_WRAPPER="${REPO_ROOT}/scripts/zigbuild-ar-wrapper.sh"
export AR_x86_64_unknown_linux_gnu="$AR_WRAPPER"
export AR_aarch64_unknown_linux_gnu="$AR_WRAPPER"

for arch in $ARCHS; do
  case "$arch" in
    x86_64)  target="x86_64-unknown-linux-gnu"  ;;
    aarch64) target="aarch64-unknown-linux-gnu" ;;
    *) echo "Unsupported arch: $arch" >&2; exit 1 ;;
  esac

  echo ""
  echo "==> building ${target}.${GLIBC}"
  cargo zigbuild --release -p garyx --target "${target}.${GLIBC}"

  staging="dist/garyx-${VERSION}-${target}"
  rm -rf "$staging"
  mkdir -p "$staging"
  cp "target/${target}/release/garyx" "$staging/"
  cp README.md LICENSE "$staging/" 2>/dev/null || true

  archive="${staging}.tar.gz"
  tar -czf "$archive" -C dist "garyx-${VERSION}-${target}"
  shasum -a 256 "$archive" | tee "${archive}.sha256"

  size="$(du -h "$staging/garyx" | awk '{print $1}')"
  echo "    -> ${archive} (binary: ${size})"
done

echo ""
echo "==> done. artifacts in dist/"
