# Pacman repository (self-hosted)

Walltag publishes Arch Linux packages to GitHub Pages on every `v*` tag.
Users add the repo once; later releases install with `pacman -Syu`.

Public repo URL layout:

```text
https://dante-ferr.github.io/walltag/x86_64/
```

## Install (Arch)

```bash
# From a clone of this repository:
sudo install -Dm644 packaging/pacman/walltag.conf /etc/pacman.d/walltag

# Add to /etc/pacman.conf (near other Include lines):
# Include = /etc/pacman.d/walltag

sudo pacman -Syu walltag
```

Packages are unsigned (`SigLevel = Optional TrustAll`). Prefer verifying the
GitHub Actions workflow that builds them if that matters for your threat model.

## How publishing works

`.github/workflows/release.yml` job `pacman-repo`:

1. Builds `packaging/aur/PKGBUILD` in an Arch container (`makepkg`).
2. Checks out (or creates) the `gh-pages` branch.
3. Adds the `.pkg.tar.zst` under `x86_64/`, runs `repo-add`, and pushes.
4. Attaches the package to the GitHub Release.

Enable GitHub Pages once: **Settings → Pages → Deploy from branch `gh-pages` / root**.
CI also tries to enable Pages via the API when it first publishes.

## Local rebuild

On Arch:

```bash
./packaging/pacman/build-and-publish-local.sh
```

That only builds into `packaging/pacman/out/` unless you pass `--publish`
(requires push access to `gh-pages`).
