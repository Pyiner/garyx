#!/usr/bin/env python3
"""Aggregate Rust test logs into machine-readable summaries.

Also supports known-failure matching for non-blocking test lanes.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

try:
    import tomllib  # py311+
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib  # type: ignore


TEST_FAILED_RE = re.compile(r"^\s*test\s+(.+?)\s+\.\.\.\s+FAILED\s*$", re.MULTILINE)
TEST_STDOUT_RE = re.compile(r"^----\s+(.+?)\s+stdout\s+----\s*$", re.MULTILINE)
COMPILE_FAIL_RE = re.compile(r"error:\s+could not compile\s+[`'\"]?([^`'\"\s]+)")


def read_text(path: Path) -> str:
    if not path.exists():
        return ""
    return path.read_text(encoding="utf-8", errors="replace")


def parse_failed_tests(log_text: str) -> list[str]:
    tests = set(TEST_FAILED_RE.findall(log_text))
    tests.update(TEST_STDOUT_RE.findall(log_text))
    return sorted(tests)


def parse_compile_failed_packages(log_text: str) -> list[str]:
    return sorted(set(COMPILE_FAIL_RE.findall(log_text)))


def load_known_failures(path: Path | None) -> list[dict[str, Any]]:
    if path is None or not path.exists():
        return []
    data = tomllib.loads(path.read_text(encoding="utf-8"))
    entries = data.get("failure", [])
    if not isinstance(entries, list):
        return []

    normalized: list[dict[str, Any]] = []
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        normalized.append(
            {
                "id": str(entry.get("id", "")),
                "package": str(entry.get("package", "*")),
                "reason": str(entry.get("reason", "")),
                "match": str(entry.get("match", "")),
                "expiry_date": str(entry.get("expiry_date", "")),
            }
        )
    return normalized


def match_known_failures(
    known_failures: list[dict[str, Any]],
    package: str,
    log_text: str,
    compile_packages: list[str],
) -> list[dict[str, Any]]:
    matched: dict[str, dict[str, Any]] = {}
    compile_package_set = set(compile_packages)

    for entry in known_failures:
        entry_package = entry.get("package", "*")
        if entry_package not in ("*", package) and entry_package not in compile_package_set:
            continue

        snippet = entry.get("match", "")
        if snippet and snippet not in log_text:
            continue

        entry_id = entry.get("id") or f"{entry_package}:{snippet[:32]}"
        matched[str(entry_id)] = entry

    return [matched[k] for k in sorted(matched)]


def parse_csv(value: str) -> list[str]:
    value = value.strip()
    if not value:
        return []
    return [part.strip() for part in value.split(",") if part.strip()]


def run_check_known(args: argparse.Namespace) -> int:
    log_path = Path(args.log_file)
    known_file = Path(args.known_file)

    log_text = read_text(log_path)
    compile_packages = parse_compile_failed_packages(log_text)
    known_failures = load_known_failures(known_file)
    matched = match_known_failures(known_failures, args.package, log_text, compile_packages)

    if args.print_ids:
        ids = [entry.get("id", "") for entry in matched if entry.get("id")]
        print(",".join(ids))
    else:
        print(len(matched))

    return 0


def run_summary(args: argparse.Namespace) -> int:
    log_path = Path(args.log_file)
    out_path = Path(args.output_file)

    log_text = read_text(log_path)

    failed_tests = parse_failed_tests(log_text)
    compile_failed_packages = parse_compile_failed_packages(log_text)

    manual_failed_packages = parse_csv(args.failed_packages)
    failed_packages = sorted(set(manual_failed_packages + compile_failed_packages))

    blocking_failures = int(args.blocking_failures)
    known_failures = int(args.known_failures)

    status = args.status
    if not status:
        if blocking_failures > 0:
            status = "fail"
        elif known_failures > 0:
            status = "warn"
        else:
            status = "pass"

    payload = {
        "tier": args.tier,
        "status": status,
        "blocking_failures": blocking_failures,
        "known_failures": known_failures,
        "failed_packages": failed_packages,
        "failed_tests": failed_tests,
        "duration_sec": int(args.duration_sec),
        "timestamp": datetime.now(UTC).isoformat(),
    }

    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Rust test summary utilities")
    parser.add_argument("--check-known", action="store_true", help="Known-failure match mode")

    parser.add_argument("--tier", default="", help="Tier name")
    parser.add_argument("--log-file", required=True, help="Path to aggregated test log")
    parser.add_argument("--output-file", default="", help="Path to output JSON summary")
    parser.add_argument("--status", default="", help="Explicit status: pass/fail/warn/skipped")
    parser.add_argument("--blocking-failures", default=0, type=int)
    parser.add_argument("--known-failures", default=0, type=int)
    parser.add_argument("--failed-packages", default="", help="Comma-separated failed package list")
    parser.add_argument("--duration-sec", default=0, type=int)

    parser.add_argument("--known-file", default="", help="Known-failures TOML file")
    parser.add_argument("--package", default="", help="Package label for known-failure match")
    parser.add_argument("--print-ids", action="store_true", help="Print matched known-failure IDs")

    return parser


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    if args.check_known:
        if not args.known_file:
            print("--known-file is required in --check-known mode", file=sys.stderr)
            return 2
        if not args.package:
            print("--package is required in --check-known mode", file=sys.stderr)
            return 2
        return run_check_known(args)

    if not args.tier:
        print("--tier is required in summary mode", file=sys.stderr)
        return 2
    if not args.output_file:
        print("--output-file is required in summary mode", file=sys.stderr)
        return 2

    return run_summary(args)


if __name__ == "__main__":
    raise SystemExit(main())
