#!/usr/bin/env bash
# Build and install the local Garyx CLI, preserving the macOS TCC identity.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SOURCE_BINARY="${REPO_ROOT}/target/release/garyx"
DESTINATIONS=()

cd "$REPO_ROOT"

add_destination() {
  local destination="$1"
  local existing

  [[ -n "$destination" ]] || return 0

  if ((${#DESTINATIONS[@]} > 0)); then
    for existing in "${DESTINATIONS[@]}"; do
      if [[ "$existing" == "$destination" ]]; then
        return 0
      fi
    done
  fi

  DESTINATIONS+=("$destination")
}

add_existing_destination() {
  local destination="$1"

  if [[ -e "$destination" ]]; then
    add_destination "$destination"
  fi
}

add_path_destinations() {
  local destination

  if ! command -v which >/dev/null 2>&1; then
    return 0
  fi

  while IFS= read -r destination; do
    add_destination "$destination"
  done < <(which -a garyx 2>/dev/null || true)
}

extract_launchd_garyx_path() {
  local launchd_command="$1"
  local escaped_path
  local quoted_path

  escaped_path="$(
    printf '%s\n' "$launchd_command" |
      grep -Eo '\\\"[^\\\"]*/garyx\\\"' |
      head -n 1 |
      sed 's/\\\"//g' || true
  )"
  if [[ -n "$escaped_path" ]]; then
    printf '%s\n' "$escaped_path"
    return 0
  fi

  quoted_path="$(
    printf '%s\n' "$launchd_command" |
      grep -Eo '"[^"]*/garyx"' |
      head -n 1 |
      tr -d '"' || true
  )"
  if [[ -n "$quoted_path" ]]; then
    printf '%s\n' "$quoted_path"
  fi
}

add_launchd_destination() {
  local plist_path="$HOME/Library/LaunchAgents/com.garyx.agent.plist"
  local launchd_command
  local launchd_path

  if [[ "$(uname -s)" != "Darwin" || ! -f "$plist_path" ]]; then
    return 0
  fi

  launchd_command="$(/usr/bin/plutil -extract ProgramArguments.2 raw -o - "$plist_path" 2>/dev/null || true)"
  launchd_path="$(extract_launchd_garyx_path "$launchd_command")"
  add_destination "$launchd_path"
}

bash scripts/build-local-cli.sh

add_destination "$HOME/.cargo/bin/garyx"
add_existing_destination "$HOME/.local/bin/garyx"
add_existing_destination "$HOME/.garyx/bin/garyx"
add_existing_destination "/opt/homebrew/bin/garyx"
add_path_destinations
add_launchd_destination

for destination in "${DESTINATIONS[@]}"; do
  mkdir -p "$(dirname "$destination")"
  install -m 755 "$SOURCE_BINARY" "$destination"
  bash scripts/codesign-macos-cli.sh "$destination"
done

echo "Installed Garyx CLI to:"
for destination in "${DESTINATIONS[@]}"; do
  echo "  $destination"
done

primary_binary="$(command -v garyx || true)"
if [[ -n "$primary_binary" ]]; then
  "$primary_binary" --version
else
  "${DESTINATIONS[0]}" --version
fi
