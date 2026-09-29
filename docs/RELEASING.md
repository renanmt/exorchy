# Releasing eXorchy

A release is a git tag. Pushing `vX.Y.Z` runs `.github/workflows/release.yml`,
which builds the pacman package in a clean `archlinux:base-devel` container
(`packaging/release/build-package.sh`: `packaging/aur/PKGBUILD` over a
`git archive` of the tag, release build, core tests in `check()`) and
publishes a GitHub release with three files:

| Asset | What it is |
|---|---|
| `exorchy-x86_64.pkg.tar.zst` | the package (stable name, so `releases/latest/download/` always finds it) |
| `exorchy-x86_64.pkg.tar.zst.sha256` | its checksum, `sha256sum -c` format |
| `install.sh` | the installer users pipe into bash (`packaging/release/install.sh`) |

Users install and update with
`curl -fsSL https://github.com/renanmt/exorchy/releases/latest/download/install.sh | bash`
(README → Install). `install.sh` downloads the package, checks the checksum
and runs `sudo pacman -U`; `--version vX.Y.Z`, `--download-only [DIR]` and
`--uninstall` are its options.

## Every release

1. **Version and changelog.** Raise `version` in the root `Cargo.toml` and
   `pkgver` in `packaging/aur/PKGBUILD` and `packaging/PKGBUILD` to the same
   number (a new version starts at `pkgrel=1`); `cargo build` refreshes
   `Cargo.lock`. Add a `## [X.Y.Z] - YYYY-MM-DD` section at the top of
   `CHANGELOG.md` (Added / Changed / Fixed, written for users): it becomes the
   release notes, and the workflow refuses a tag without one. Preview it with
   `packaging/release/changelog-section.sh X.Y.Z`. Commit, open a PR, merge.
2. **Dry run (optional).** GitHub → Actions → Release → *Run workflow* on
   `main`: it builds exactly as a release would and keeps the three files as
   an artifact, without publishing anything.
3. **Tag.** On an up-to-date `main`:

   ```bash
   git switch main && git pull
   git tag -a v0.2.0 -m "eXorchy 0.2.0"
   git push origin v0.2.0
   ```

   The workflow refuses a tag that does not match `Cargo.toml` and the
   PKGBUILD, or that has no `CHANGELOG.md` section. It takes about ten
   minutes; the release appears under *Releases* with that section as its
   notes, followed by the install instructions. eXorchy's update banner links
   there ("What's new").
4. **Check.** On a machine with eXorchy installed, run the install line: it
   should update to the new version (`pacman -Q exorchy`).

A broken release: delete it and its tag on GitHub
(`gh release delete v0.2.0 --cleanup-tag`), fix, and tag again.

## Building the package locally

```bash
packaging/release/build-package.sh dist        # dist/: package, .sha256, install.sh
MAKEPKG_FLAGS=-d packaging/release/build-package.sh dist   # Rust from rustup, not pacman
```

It packages `HEAD`, not uncommitted changes (like the release does). To try
the installer against that output without GitHub:

```bash
(cd dist && python -m http.server 8765 &)
EXORCHY_RELEASE_URL=http://127.0.0.1:8765 bash dist/install.sh --download-only /tmp/x
```

## The AUR (when registrations reopen)

`packaging/aur/PKGBUILD` is already an AUR package: it downloads the tag's
source tarball from GitHub (`cargo fetch --locked` in `prepare()`, `--frozen`
build).

Once:

1. Register at https://aur.archlinux.org/register and add your SSH public key
   under *My Account* (`ssh-keygen -t ed25519` if you have none).
2. In `~/.ssh/config`:

   ```
   Host aur.archlinux.org
     IdentityFile ~/.ssh/id_ed25519
     User aur
   ```

3. `sudo pacman -S --needed base-devel devtools pacman-contrib namcap`

Every release, after the tag exists:

```bash
git clone ssh://aur@aur.archlinux.org/exorchy.git ~/aur/exorchy   # first time; later: git pull
cd ~/aur/exorchy
cp ~/Projects/exorchy/packaging/aur/PKGBUILD .
updpkgsums                          # fills sha256sums from the tag's tarball
makepkg --printsrcinfo > .SRCINFO
makepkg -si && pkgctl build && namcap PKGBUILD *.pkg.tar.zst
git add PKGBUILD .SRCINFO && git commit -m "Update to 0.2.0" && git push
```

## Notes

- `makepkg` warns "Package contains reference to $srcdir": the binary keeps
  its build path (`CARGO_MANIFEST_DIR`) for the source-checkout lookups. It
  only uses that path when the executable runs from inside the checkout, so a
  packaged binary never reads from a leftover build directory.
- The package targets current Arch. A system that has not updated in a long
  time may be asked by pacman to update first.
