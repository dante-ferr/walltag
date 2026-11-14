# Flatpak / Flathub packaging

Manifest: `io.github.dante_ferr.Walltag.yml`

Walltag sets wallpapers through host tools (`swww` or `awww`) and may run a
user `post_command`. Inside Flatpak those must run on the host via
`flatpak-spawn --host`, which needs `--talk-name=org.freedesktop.Flatpak`.

The manifest always uses a **git tag source** (not a parent `dir`). Sandboxed
builders such as `flathub-build` reject `path: ../..`.

## Local build

```bash
flatpak install -y flathub org.flatpak.Builder
flatpak remote-add --if-not-exists --user flathub \
  https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install -y flathub \
  org.gnome.Platform//51 \
  org.gnome.Sdk//51 \
  org.freedesktop.Sdk.Extension.rust-stable//26.08

# After Cargo.lock changes (and before a new release tag):
./packaging/flatpak/generate-cargo-sources.sh

cd packaging/flatpak
flatpak run --command=flathub-build org.flatpak.Builder --install \
  io.github.dante_ferr.Walltag.yml
flatpak run io.github.dante_ferr.Walltag
```

## Lint

```bash
cd packaging/flatpak
flatpak run --command=flatpak-builder-lint org.flatpak.Builder \
  manifest io.github.dante_ferr.Walltag.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

## Flathub submission

1. Confirm local build + lint pass.
2. Fork `flathub/flathub`, branch from `new-pr`, add this manifest and
   `cargo-sources.json`.
3. Open a PR titled `Add io.github.dante_ferr.Walltag` against base `new-pr`.
4. Explain host spawning and `--filesystem=home` in the PR body.
5. After approval, accept the invite to `flathub/io.github.dante_ferr.Walltag`.

Docs: https://docs.flathub.org/docs/for-app-authors/submission

## App ID

`io.github.dante_ferr.Walltag` matches the GitHub namespace
(`dante-ferr` → `dante_ferr`) and the GTK application id.

## Shipping a new version (after Flathub accepts the app)

This upstream repo does **not** auto-publish Flatpak or crates.io. Tagging
`v*` only creates a **GitHub Release** (see `.github/workflows/release.yml`).

For each new version:

1. Bump `version` in `Cargo.toml` and the `<release>` entry in
   `assets/io.github.dante_ferr.Walltag.metainfo.xml`.
2. If `Cargo.lock` changed, run `./packaging/flatpak/generate-cargo-sources.sh`.
3. Commit, tag `vX.Y.Z`, push branch + tag (GitHub Release is automatic).
4. `cargo publish` (manual; needs crates.io token).
5. In the **Flathub app repo** (`flathub/io.github.dante_ferr.Walltag`), update
   `tag` / `commit` in the manifest, copy the new `cargo-sources.json`, commit,
   and push. Flathub builds and publishes from that repo.
6. When AUR is available: bump `pkgver` / checksums in `packaging/aur/` and push
   the AUR git repo.
