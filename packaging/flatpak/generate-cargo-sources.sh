#!/usr/bin/env bash
# Regenerate cargo-sources.json from Cargo.lock for Flatpak offline builds.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/packaging/flatpak/cargo-sources.json"
lock="$root/Cargo.lock"
generator_url="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL "$generator_url" -o "$tmp/flatpak-cargo-generator.py"

if command -v uv >/dev/null 2>&1; then
  uv run --with aiohttp --with tomlkit --with 'PyYAML>=6' \
    python "$tmp/flatpak-cargo-generator.py" "$lock" -o "$out"
else
  python3 -m pip install --user aiohttp tomlkit 'PyYAML>=6'
  python3 "$tmp/flatpak-cargo-generator.py" "$lock" -o "$out"
fi

echo "Wrote $out"
