# Flatpak / Flathub packaging

Manifest: `io.github.dante_ferr.Walltag.yml`

Walltag sets wallpapers through host tools (`swww` or `awww`) and may run a
user `post_command`. Inside Flatpak those must run on the host via
`flatpak-spawn --host`, which needs `--talk-name=org.freedesktop.Flatpak`.

## Local build

```bash
# Runtime + Rust SDK (branch matches the Freedesktop base of GNOME 48)
flatpak install -y flathub \
  org.gnome.Platform//48 \
  org.gnome.Sdk//48 \
  org.freedesktop.Sdk.Extension.rust-stable//24.08

# After Cargo.lock changes:
./packaging/flatpak/generate-cargo-sources.sh

cd packaging/flatpak
flatpak-builder --user --force-clean --install-deps-from=flathub --install \
  build-dir io.github.dante_ferr.Walltag.yml
flatpak run io.github.dante_ferr.Walltag
```

The committed manifest uses a `dir` source of this repo for local builds.

## Flathub submission

1. Tag and push `v0.1.0` on GitHub.
2. Copy the manifest into a new Flathub app repository. Replace the `dir` source with:

   ```yaml
   - type: git
     url: https://github.com/dante-ferr/walltag.git
     tag: v0.1.0
     commit: <sha-of-that-tag>
   - cargo-sources.json
   ```

3. Ensure `cargo-sources.json` matches `Cargo.lock` at that tag.
4. Follow https://docs.flathub.org/docs/for-app-authors/submission
5. In the submission, explain host spawning: wallpaper daemons and
   `post_command` are host programs; the app does not ship them in the sandbox.

Flathub may ask for tighter permissions later. Keep `--filesystem=home` so users
can open wallpaper folders under `$HOME`.

## App ID

`io.github.dante_ferr.Walltag` matches the GitHub namespace
(`dante-ferr` → `dante_ferr`) and the GTK application id.
