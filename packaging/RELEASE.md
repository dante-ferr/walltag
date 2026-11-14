# Release process

Tagging `v*` runs [`.github/workflows/release.yml`](../.github/workflows/release.yml).
That workflow creates the GitHub Release and, when secrets are configured,
publishes to crates.io and opens a Flathub update PR.

## Before every tag

1. Bump `version` in `Cargo.toml`.
2. Add a `<release>` entry in `assets/io.github.dante_ferr.Walltag.metainfo.xml`.
3. Update `packaging/github/RELEASE_NOTES.md` for the GitHub Release body.
4. Update `tag` / `commit` in `packaging/flatpak/io.github.dante_ferr.Walltag.yml`
   to the commit you are about to tag (or leave them for the Flathub job to
   rewrite in the Flathub app repo).
5. If `Cargo.lock` changed since the last release, regenerate Flatpak cargo
   sources and commit the result:

   ```bash
   ./packaging/flatpak/generate-cargo-sources.sh
   git add packaging/flatpak/cargo-sources.json
   ```

6. Commit on `master` (conventional commits), then tag and push:

   ```bash
   git tag -a vX.Y.Z -m "vX.Y.Z"
   git push origin master
   git push origin vX.Y.Z
   ```

## What the workflow automates

| Step | Secret | Behavior when missing |
| --- | --- | --- |
| GitHub Release | (none; uses `GITHUB_TOKEN`) | Always runs |
| `cargo publish --locked` | `CRATES_IO_TOKEN` | Skips with a notice and prints setup steps |
| Check / regenerate `cargo-sources.json` | (none) | Always runs; warns if the committed file is stale |
| Flathub update PR | `FLATHUB_TOKEN` | Skips with a notice and prints manual git steps |

## One-time secrets (GitHub → Settings → Secrets and variables → Actions)

### `CRATES_IO_TOKEN`

1. Create a crates.io account and verify email: https://crates.io
2. Create an API token: https://crates.io/settings/tokens
3. Add the token as repository secret `CRATES_IO_TOKEN`.

Never commit the token. The workflow maps it to `CARGO_REGISTRY_TOKEN`.

### `FLATHUB_TOKEN`

Required only after Flathub accepts the app and invites you to
`flathub/io.github.dante_ferr.Walltag`.

1. Accept the GitHub invite to that repository.
2. Create a GitHub personal access token that can push and open PRs on
   `flathub/io.github.dante_ferr.Walltag`:
   - Classic: `repo` scope
   - Fine-grained: Contents (read/write) and Pull requests (read/write) on
     that repository
3. Add the token as repository secret `FLATHUB_TOKEN`.

The workflow clones the Flathub app repo, updates the manifest `tag` /
`commit`, copies `cargo-sources.json`, pushes branch `update-vX.Y.Z`, and
opens or updates a PR. Merge that PR to publish on Flathub.

## Still manual (cannot fully automate)

### First Flathub submission

Until the app exists under `flathub/io.github.dante_ferr.Walltag`, updates
cannot be pushed there. Do the submission once:

1. Confirm a local Flatpak build and lint pass (see
   [`packaging/flatpak/README.md`](flatpak/README.md)).
2. Fork `flathub/flathub`, branch from `new-pr`, add
   `io.github.dante_ferr.Walltag.yml` and `cargo-sources.json`.
3. Open a PR titled `Add io.github.dante_ferr.Walltag` against base `new-pr`.
4. Explain host spawning (`flatpak-spawn --host`) and `--filesystem=home` in
   the PR body.
5. After approval, accept the invite to `flathub/io.github.dante_ferr.Walltag`
   and add `FLATHUB_TOKEN` as above.

Docs: https://docs.flathub.org/docs/for-app-authors/submission

### Manual Flathub update (no `FLATHUB_TOKEN`)

```bash
git clone git@github.com:flathub/io.github.dante_ferr.Walltag.git
cd io.github.dante_ferr.Walltag
# Copy packaging/flatpak/io.github.dante_ferr.Walltag.yml and
# packaging/flatpak/cargo-sources.json from the release tag, then set:
#   tag: vX.Y.Z
#   commit: <full sha of that tag>
git commit -am "Update Walltag to vX.Y.Z"
git push origin master   # or open a PR
```

### AUR

AUR account registration is periodically closed. When it reopens:

1. Follow [`packaging/aur/README.md`](aur/README.md) for the first upload.
2. For later releases: bump `pkgver` / checksums, regenerate `.SRCINFO`, push
   to `ssh://aur@aur.archlinux.org/walltag.git`.

There is no AUR API suitable for this repo’s CI; keep AUR updates manual.

### crates.io without the secret

```bash
cargo login
cargo publish --dry-run --locked
cargo publish --locked
```

## Verify after a release

1. GitHub Release appears for the tag.
2. If `CRATES_IO_TOKEN` is set: https://crates.io/crates/walltag shows the
   new version.
3. If `FLATHUB_TOKEN` is set: a PR exists on
   `flathub/io.github.dante_ferr.Walltag`; merge it and confirm the Flathub
   build.
4. Optionally bump and push AUR packaging when registration allows.
