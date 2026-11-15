#!/usr/bin/env bash
# Build the Arch package from packaging/aur/PKGBUILD into packaging/pacman/out.
# With --publish, also update the local gh-pages worktree layout under out/repo.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
aur="$root/packaging/aur"
out="$root/packaging/pacman/out"
publish=false

for arg in "$@"; do
  case "$arg" in
    --publish) publish=true ;;
    -h|--help)
      echo "Usage: $0 [--publish]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 1
      ;;
  esac
done

if ! command -v makepkg >/dev/null; then
  echo "makepkg not found; run this on Arch Linux." >&2
  exit 1
fi

rm -rf "$out"
mkdir -p "$out"
cp "$aur/PKGBUILD" "$out/PKGBUILD"
# Copy .SRCINFO if present (optional for makepkg)
[[ -f "$aur/.SRCINFO" ]] && cp "$aur/.SRCINFO" "$out/.SRCINFO"

(
  cd "$out"
  # Keep build artifacts inside the package source tree (ignore env CARGO_TARGET_DIR).
  unset CARGO_TARGET_DIR
  export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
  # -d: skip pacman deps when rustc/cargo come from rustup instead of pacman.
  makepkg -df --noconfirm --cleanbuild
)

pkg="$(find "$out" -maxdepth 1 -name 'walltag-*.pkg.tar.zst' | head -n1)"
if [[ -z "$pkg" ]]; then
  echo "No package produced under $out" >&2
  exit 1
fi

echo "Built $pkg"

if [[ "$publish" != true ]]; then
  exit 0
fi

repo_root="$out/repo"
mkdir -p "$repo_root/x86_64"
cp -f "$pkg" "$repo_root/x86_64/"
cp -f "$root/packaging/pacman/walltag.conf" "$repo_root/walltag.conf"
cp -f "$root/packaging/pacman/index.html" "$repo_root/index.html"
(
  cd "$repo_root/x86_64"
  repo-add -n -R walltag.db.tar.zst ./*.pkg.tar.zst
)
echo "Repo tree ready at $repo_root (push this tree to the gh-pages branch)."
