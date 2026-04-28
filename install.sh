#!/usr/bin/env bash
# Garyx installer — downloads the latest release binary with checksum verification.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Pyiner/garyx/main/install.sh | bash
#
# Options (env vars):
#   GARYX_VERSION   — specific version to install (default: latest)
#   GARYX_INSTALL   — installation directory (default: ~/.garyx/bin)

set -euo pipefail

REPO="Pyiner/garyx"
INSTALL_DIR="${GARYX_INSTALL:-$HOME/.garyx/bin}"

main() {
  check_deps

  local version="${GARYX_VERSION:-}"
  local target

  target="$(detect_target)"
  echo "Detected platform: ${target}"

  if [ -z "$version" ]; then
    version="$(latest_version)"
  fi
  if [ -z "$version" ]; then
    die "Could not determine latest version. Set GARYX_VERSION or check https://github.com/${REPO}/releases"
  fi
  echo "Installing garyx ${version}..."

  local base_url="https://github.com/${REPO}/releases/download/v${version}"
  local archive="garyx-${version}-${target}.tar.gz"

  local tmpdir
  tmpdir="$(mktemp -d)"
  trap 'rm -rf "$tmpdir"' EXIT

  echo "Downloading ${archive}..."
  curl -fsSL "${base_url}/${archive}" -o "${tmpdir}/${archive}"
  curl -fsSL "${base_url}/${archive}.sha256" -o "${tmpdir}/${archive}.sha256"

  echo "Verifying checksum..."
  verify_checksum "${tmpdir}/${archive}" "${tmpdir}/${archive}.sha256"

  tar xzf "${tmpdir}/${archive}" -C "$tmpdir"

  mkdir -p "$INSTALL_DIR"
  cp "${tmpdir}/garyx-${version}-${target}/garyx" "$INSTALL_DIR/"
  chmod +x "$INSTALL_DIR/garyx"

  echo ""
  echo "Installed garyx to ${INSTALL_DIR}/garyx"
  echo ""

  if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    echo "Add to your PATH:"
    echo ""
    echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo ""
  fi

  echo "Run 'garyx --version' to verify."
}

check_deps() {
  for cmd in curl tar mktemp; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
      die "Required command '${cmd}' not found. Please install it first."
    fi
  done
}

detect_target() {
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

latest_version() {
  local ver
  ver="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | grep '"tag_name"' \
    | head -1 \
    | sed 's/.*"v\([^"]*\)".*/\1/')" || true
  echo "$ver"
}

verify_checksum() {
  local file="$1" sha_file="$2"
  local expected actual

  expected="$(awk '{print $1}' "$sha_file")"
  if [ -z "$expected" ]; then
    die "Checksum file is empty or malformed."
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$file" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  else
    die "sha256sum or shasum is required for checksum verification."
  fi

  if [ "$expected" != "$actual" ]; then
    die "Checksum mismatch!\n  Expected: ${expected}\n  Actual:   ${actual}\nThe download may be corrupted."
  fi
  echo "Checksum OK."
}

die() {
  echo "Error: $*" >&2
  exit 1
}

main "$@"
