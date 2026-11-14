# AUR packaging for Walltag

These files are the source of truth for the [AUR](https://wiki.archlinux.org/title/Arch_User_Repository) package `walltag`. The AUR git repository should contain only `PKGBUILD`, `.SRCINFO`, and optional install hooks—not this README.

## One-time setup

1. [Create an AUR account](https://aur.archlinux.org/register) if you do not have one.
2. Add an SSH public key under **My account → SSH public keys**.
3. On GitHub, ensure release `v0.1.0` (or the current `pkgver`) exists: tag `v0.1.0` and push it so the `source` URL in `PKGBUILD` resolves.

## Refresh checksums and `.SRCINFO`

From this directory, after the GitHub tag exists:

```bash
updpkgsums
makepkg --printsrcinfo > .SRCINFO
```

Commit both `PKGBUILD` and `.SRCINFO` when either changes.

## Test the package locally

```bash
makepkg -src
# optional install on your machine:
makepkg -si
```

`makepkg -s` installs `makedepends` via pacman; you need network and sudo for `-i`.

## First upload to AUR

```bash
git clone ssh://aur@aur.archlinux.org/walltag.git /tmp/aur-walltag
cp PKGBUILD .SRCINFO /tmp/aur-walltag/
cd /tmp/aur-walltag
git add PKGBUILD .SRCINFO
git commit -m "Initial commit: walltag 0.1.0"
git push origin master
```

If the package name is taken, pick another name or contact the existing maintainer.

## Later releases

1. Bump `pkgver` / `pkgrel` in `PKGBUILD`.
2. Tag the matching release on GitHub (`v$pkgver`).
3. Run `updpkgsums` and regenerate `.SRCINFO`.
4. Copy files into your AUR clone, commit, and push.

## Install for users

After the package is on AUR:

```bash
git clone https://aur.archlinux.org/walltag.git
cd walltag
makepkg -si
```

Or use an AUR helper (`yay -S walltag`, `paru -S walltag`, etc.).