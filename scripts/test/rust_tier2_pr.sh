#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
RUST_DIR="$ROOT_DIR"
SUMMARY_PY="$SCRIPT_DIR/rust_test_summary.py"
KNOWN_FILE="$RUST_DIR/tests/known-failures.toml"

ALLOW_KNOWN_FAILURES="${ALLOW_KNOWN_FAILURES:-1}"
FAIL_FAST="${RUST_TEST_FAIL_FAST:-0}"

REPORT_DIR="$ROOT_DIR/target/test-reports"
LOG_DIR="$REPORT_DIR/logs"
LOG_FILE="$LOG_DIR/tier2.log"
REPORT_JSON="$REPORT_DIR/tier2.json"
mkdir -p "$LOG_DIR"
: > "$LOG_FILE"
rm -f "$REPORT_JSON"

start_ts="$(date +%s)"
blocking_failures=0
known_failures=0

FAILED_PACKAGES=()

run_command() {
  local label="$1"
  local cmd="$2"
  local cmd_log="$LOG_DIR/tier2_${label}.log"
  local rc

  echo "=== [$label] $cmd" | tee -a "$LOG_FILE"
  if (cd "$RUST_DIR" && bash -lc "$cmd") >"$cmd_log" 2>&1; then
    rc=0
  else
    rc=$?
  fi

  cat "$cmd_log" >> "$LOG_FILE"

  if [[ $rc -eq 0 ]]; then
    return 0
  fi

  FAILED_PACKAGES+=("$label")

  if [[ "$ALLOW_KNOWN_FAILURES" == "1" ]]; then
    local known_match_count
    known_match_count="$(python3 "$SUMMARY_PY" --check-known --known-file "$KNOWN_FILE" --package "$label" --log-file "$cmd_log")"
    if [[ "$known_match_count" =~ ^[0-9]+$ ]] && [[ "$known_match_count" -gt 0 ]]; then
      known_failures=$((known_failures + 1))
      local known_ids
      known_ids="$(python3 "$SUMMARY_PY" --check-known --known-file "$KNOWN_FILE" --package "$label" --log-file "$cmd_log" --print-ids)"
      echo "[tier2] known failure matched for $label: ${known_ids:-<unknown-id>}" | tee -a "$LOG_FILE"
      return 0
    fi
  fi

  blocking_failures=$((blocking_failures + 1))
  echo "[tier2] blocking failure in package: $label" | tee -a "$LOG_FILE"

  if [[ "$FAIL_FAST" == "1" ]]; then
    return 99
  fi

  return 1
}

set +e
run_command "workspace-core" "cargo test --workspace --all-targets --exclude garyx-gateway --exclude garyx"
rc=$?
set -e
if [[ "$rc" -eq 99 ]]; then
  end_ts="$(date +%s)"
  duration_sec=$((end_ts - start_ts))
  status="fail"
  failed_packages_csv="$(IFS=,; echo "${FAILED_PACKAGES[*]}")"
  python3 "$SUMMARY_PY" \
    --tier "tier2" \
    --log-file "$LOG_FILE" \
    --output-file "$REPORT_JSON" \
    --status "$status" \
    --blocking-failures "$blocking_failures" \
    --known-failures "$known_failures" \
    --failed-packages "$failed_packages_csv" \
    --duration-sec "$duration_sec"
  echo "TIER=tier2 STATUS=$status BLOCKING_FAILURES=$blocking_failures KNOWN_FAILURES=$known_failures DURATION_SEC=$duration_sec"
  exit 1
fi

set +e
run_command "garyx-gateway" "cargo test -p garyx-gateway"
rc=$?
set -e

set +e
run_command "garyx" "cargo test -p garyx"
rc=$?
set -e

end_ts="$(date +%s)"
duration_sec=$((end_ts - start_ts))

status="pass"
if [[ "$blocking_failures" -gt 0 ]]; then
  status="fail"
elif [[ "$known_failures" -gt 0 ]]; then
  status="warn"
fi

failed_packages_csv=""
if [[ "${#FAILED_PACKAGES[@]}" -gt 0 ]]; then
  failed_packages_csv="$(IFS=,; echo "${FAILED_PACKAGES[*]}")"
fi

python3 "$SUMMARY_PY" \
  --tier "tier2" \
  --log-file "$LOG_FILE" \
  --output-file "$REPORT_JSON" \
  --status "$status" \
  --blocking-failures "$blocking_failures" \
  --known-failures "$known_failures" \
  --failed-packages "$failed_packages_csv" \
  --duration-sec "$duration_sec"

echo "TIER=tier2 STATUS=$status BLOCKING_FAILURES=$blocking_failures KNOWN_FAILURES=$known_failures DURATION_SEC=$duration_sec"

if [[ "$status" == "fail" ]]; then
  exit 1
fi
