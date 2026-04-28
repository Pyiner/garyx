#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
RUST_DIR="$ROOT_DIR"
SUMMARY_PY="$SCRIPT_DIR/rust_test_summary.py"

MODE="--all"
if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [--changed|--all]" >&2
  exit 2
fi
if [[ $# -eq 1 ]]; then
  case "$1" in
    --changed|--all)
      MODE="$1"
      ;;
    *)
      echo "Usage: $0 [--changed|--all]" >&2
      exit 2
      ;;
  esac
fi

REPORT_DIR="$ROOT_DIR/target/test-reports"
LOG_DIR="$REPORT_DIR/logs"
LOG_FILE="$LOG_DIR/tier1.log"
REPORT_JSON="$REPORT_DIR/tier1.json"
mkdir -p "$LOG_DIR"
: > "$LOG_FILE"

start_ts="$(date +%s)"
blocking_failures=0
known_failures=0

TIER1_PACKAGES=(
  "garyx-models"
  "garyx-core"
  "garyx-router"
  "garyx-bridge"
  "garyx-channels"
  "garyx-gateway"
)

contains_pkg() {
  local needle="$1"
  shift
  local item
  for item in "$@"; do
    if [[ "$item" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

command_for_pkg() {
  case "$1" in
    garyx-models) echo "cargo test -p garyx-models --all-targets" ;;
    garyx-core) echo "cargo test -p garyx-core --all-targets" ;;
    garyx-router) echo "cargo test -p garyx-router --all-targets" ;;
    garyx-bridge) echo "cargo test -p garyx-bridge --all-targets" ;;
    garyx-channels) echo "cargo test -p garyx-channels --lib" ;;
    garyx-gateway) echo "cargo test -p garyx-gateway --lib" ;;
    *)
      echo "Unknown package: $1" >&2
      return 1
      ;;
  esac
}

resolve_changed_targets() {
  local changed_file
  local all_needed=0
  local -a resolved=()

  while IFS= read -r changed_file; do
    [[ -z "$changed_file" ]] && continue

    case "$changed_file" in
      Cargo.toml|Cargo.lock)
        all_needed=1
        ;;
      */Cargo.toml)
        all_needed=1
        ;;
      *)
        local crate
        crate="${changed_file#}"
        crate="${crate%%/*}"
        if contains_pkg "$crate" "${TIER1_PACKAGES[@]}"; then
          if [[ "${#resolved[@]}" -eq 0 ]] || ! contains_pkg "$crate" "${resolved[@]}"; then
            resolved+=("$crate")
          fi
        fi
        ;;
    esac
  done < <(
    {
      git -C "$ROOT_DIR" diff --name-only HEAD
      git -C "$ROOT_DIR" ls-files --others --exclude-standard
    } | sort -u
  )

  if [[ "$all_needed" -eq 1 ]]; then
    printf '%s\n' "${TIER1_PACKAGES[@]}"
    return
  fi

  if [[ "${#resolved[@]}" -eq 0 ]]; then
    return
  fi

  printf '%s\n' "${resolved[@]}"
}

run_test_command() {
  local label="$1"
  local cmd="$2"
  local fail_fast="${RUST_TEST_FAIL_FAST:-0}"

  echo "=== [$label] $cmd" | tee -a "$LOG_FILE"
  set +e
  (cd "$RUST_DIR" && bash -lc "$cmd") >>"$LOG_FILE" 2>&1
  local rc=$?
  set -e

  if [[ $rc -ne 0 ]]; then
    blocking_failures=$((blocking_failures + 1))
    FAILED_PACKAGES+=("$label")
    echo "[tier1] failed package: $label" | tee -a "$LOG_FILE"
    if [[ "$fail_fast" == "1" ]]; then
      return 99
    fi
  fi
  return 0
}

FAILED_PACKAGES=()
TARGET_PACKAGES=()

if [[ "$MODE" == "--all" ]]; then
  TARGET_PACKAGES=("${TIER1_PACKAGES[@]}")
else
  while IFS= read -r pkg; do
    [[ -n "$pkg" ]] && TARGET_PACKAGES+=("$pkg")
  done < <(resolve_changed_targets)
fi

if [[ "${#TARGET_PACKAGES[@]}" -eq 0 ]]; then
  echo "[tier1] no Rust crate changes detected; skipping command execution" | tee -a "$LOG_FILE"
else
  for pkg in "${TARGET_PACKAGES[@]}"; do
    cmd="$(command_for_pkg "$pkg")"
    set +e
    run_test_command "$pkg" "$cmd"
    rc=$?
    set -e
    if [[ "$rc" -eq 99 ]]; then
      break
    fi
  done
fi

end_ts="$(date +%s)"
duration_sec=$((end_ts - start_ts))
status="pass"
if [[ "$blocking_failures" -gt 0 ]]; then
  status="fail"
fi

failed_packages_csv=""
if [[ "${#FAILED_PACKAGES[@]}" -gt 0 ]]; then
  failed_packages_csv="$(IFS=,; echo "${FAILED_PACKAGES[*]}")"
fi

python3 "$SUMMARY_PY" \
  --tier "tier1" \
  --log-file "$LOG_FILE" \
  --output-file "$REPORT_JSON" \
  --status "$status" \
  --blocking-failures "$blocking_failures" \
  --known-failures "$known_failures" \
  --failed-packages "$failed_packages_csv" \
  --duration-sec "$duration_sec"

echo "TIER=tier1 STATUS=$status BLOCKING_FAILURES=$blocking_failures KNOWN_FAILURES=$known_failures DURATION_SEC=$duration_sec"

if [[ "$status" == "fail" ]]; then
  exit 1
fi
