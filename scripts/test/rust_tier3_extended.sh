#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
RUST_DIR="$ROOT_DIR"
SUMMARY_PY="$SCRIPT_DIR/rust_test_summary.py"

RUN_EXTERNAL_AI_TESTS="${RUN_EXTERNAL_AI_TESTS:-0}"

REPORT_DIR="$ROOT_DIR/target/test-reports"
LOG_DIR="$REPORT_DIR/logs"
LOG_FILE="$LOG_DIR/tier3.log"
REPORT_JSON="$REPORT_DIR/tier3.json"
mkdir -p "$LOG_DIR"
: > "$LOG_FILE"

start_ts="$(date +%s)"
blocking_failures=0
known_failures=0
FAILED_PACKAGES=()

emit_summary() {
  local status="$1"
  local end_ts
  local duration_sec
  local failed_packages_csv=""

  end_ts="$(date +%s)"
  duration_sec=$((end_ts - start_ts))

  if [[ "${#FAILED_PACKAGES[@]}" -gt 0 ]]; then
    failed_packages_csv="$(IFS=,; echo "${FAILED_PACKAGES[*]}")"
  fi

  python3 "$SUMMARY_PY" \
    --tier "tier3" \
    --log-file "$LOG_FILE" \
    --output-file "$REPORT_JSON" \
    --status "$status" \
    --blocking-failures "$blocking_failures" \
    --known-failures "$known_failures" \
    --failed-packages "$failed_packages_csv" \
    --duration-sec "$duration_sec"

  echo "TIER=tier3 STATUS=$status BLOCKING_FAILURES=$blocking_failures KNOWN_FAILURES=$known_failures DURATION_SEC=$duration_sec"
}

if [[ "$RUN_EXTERNAL_AI_TESTS" != "1" ]]; then
  echo "[tier3] skipped: set RUN_EXTERNAL_AI_TESTS=1 to run external integration tests and coverage report" | tee -a "$LOG_FILE"
  emit_summary "skipped"
  exit 0
fi

run_non_blocking() {
  local label="$1"
  local cmd="$2"

  echo "=== [$label] $cmd" | tee -a "$LOG_FILE"
  set +e
  (cd "$RUST_DIR" && bash -lc "$cmd") >>"$LOG_FILE" 2>&1
  local rc=$?
  set -e

  if [[ $rc -ne 0 ]]; then
    known_failures=$((known_failures + 1))
    FAILED_PACKAGES+=("$label")
    echo "[tier3] non-blocking failure: $label" | tee -a "$LOG_FILE"
  fi
}

run_non_blocking "claude-agent-sdk-integration" "cargo test -p claude-agent-sdk --test integration -- --ignored"
run_non_blocking "codex-sdk-integration" "cargo test -p codex-sdk --test integration -- --ignored"
run_non_blocking "garyx-bridge-integration" "cargo test -p garyx-bridge --test integration -- --ignored"
run_non_blocking "garyx-gateway-downstream-real-tests" "cargo test -p garyx-gateway --lib --features real-provider-tests downstream_real_tests -- --nocapture"
run_non_blocking "garyx-gateway-managed-mcp-real-tests" "cargo test -p garyx-gateway --lib --features real-provider-tests managed_mcp_real_tests -- --nocapture"
if (cd "$RUST_DIR" && cargo llvm-cov --version >/dev/null 2>&1); then
  run_non_blocking "coverage-llvm-cov" "cargo llvm-cov --workspace --all-targets --exclude garyx-gateway --exclude garyx --lcov --output-path target/coverage/lcov.info"
else
  known_failures=$((known_failures + 1))
  FAILED_PACKAGES+=("coverage-llvm-cov")
  echo "[tier3] non-blocking warning: cargo llvm-cov is not installed" | tee -a "$LOG_FILE"
fi

status="pass"
if [[ "$known_failures" -gt 0 ]]; then
  status="warn"
fi

emit_summary "$status"
exit 0
