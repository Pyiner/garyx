#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
scratch="$(mktemp -d /tmp/garyx-claude-session-store-parity.XXXXXX)"
cleanup() {
  rm -rf "${scratch:?}"
}
trap cleanup EXIT

official_repo="$scratch/claude-agent-sdk-typescript"
git clone --quiet --depth 1 --branch v0.3.217 \
  https://github.com/anthropics/claude-agent-sdk-typescript.git \
  "$official_repo"

expected_commit="2997b3d35a729ef823d4edf6cf3c690f86d888e3"
observed_commit="$(git -C "$official_repo" rev-parse HEAD)"
if [[ "$observed_commit" != "$expected_commit" ]]; then
  echo "official TypeScript SDK commit mismatch: $observed_commit" >&2
  exit 1
fi

npm pack @anthropic-ai/claude-agent-sdk@0.3.217 \
  --pack-destination "$scratch" --silent >/dev/null
package_archive="$(find "$scratch" -maxdepth 1 -name '*claude-agent-sdk-0.3.217.tgz' -print -quit)"
if [[ -z "$package_archive" ]]; then
  echo "could not locate pinned TypeScript SDK package" >&2
  exit 1
fi
mkdir "$scratch/package"
tar -xzf "$package_archive" -C "$scratch/package"
package_version="$(node -p "require('$scratch/package/package/package.json').version")"
if [[ "$package_version" != "0.3.217" ]]; then
  echo "official TypeScript SDK package mismatch: $package_version" >&2
  exit 1
fi

cargo build --quiet --manifest-path "$repo_root/Cargo.toml" \
  -p claude-agent-sdk --example session_store_contract \
  --example session_store_resume_probe \
  --example session_store_batch_probe

CLAUDE_TS_SDK_REPO="$official_repo" \
CLAUDE_TS_SDK_MODULE="$scratch/package/package/sdk.mjs" \
CLAUDE_RUST_SESSION_STORE_ORACLE="$repo_root/target/debug/examples/session_store_contract" \
CLAUDE_RUST_SESSION_STORE_RESUME_PROBE="$repo_root/target/debug/examples/session_store_resume_probe" \
CLAUDE_RUST_SESSION_STORE_BATCH_PROBE="$repo_root/target/debug/examples/session_store_batch_probe" \
CLAUDE_SESSION_STORE_FAKE_CLI="$repo_root/claude-agent-sdk/tests/fixtures/session_mirror_fake_cli.mjs" \
CLAUDE_SESSION_STORE_NODE="$(command -v node)" \
  bun test "$repo_root/claude-agent-sdk/tests/session_store_typescript_parity.test.ts"
