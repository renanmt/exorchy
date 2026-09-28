# Releasing eXorchy to the AUR

The AUR package is `exorchy`, built from the GitHub release tarball by
`packaging/aur/PKGBUILD` (release build, core tests in `check()`, resources
under `/usr/lib/exorchy`). `packaging/PKGBUILD` is the other one: it builds
the local checkout, for testing.

## Once: AUR account

1. Register at https://aur.archlinux.org/register (username, e-mail).
2. Add your SSH public key under *My Account → SSH Public Key*
   (`cat ~/.ssh/id_ed25519.pub`; create one with `ssh-keygen -t ed25519` if
   needed).
3. Tell SSH which key the AUR uses, in `~/.ssh/config`:

   ```
   Host aur.archlinux.org
     IdentityFile ~/.ssh/id_ed25519
     User aur
   ```

4. Install the tools: `sudo pacman -S --needed base-devel devtools pacman-contrib namcap`.

## Every release

1. **Version.** Raise `version` in the root `Cargo.toml` and `pkgver` in both
   PKGBUILDs to the same number; `cargo build` to refresh `Cargo.lock`.
   Commit, push, merge to `main`.
2. **Tag.** On an up-to-date `main`:

   ```bash
   git tag -a v0.2.0 -m "eXorchy 0.2.0"
   git push origin v0.2.0
   ```

   Optionally create a GitHub release from the tag (notes, screenshots). The
   tarball the PKGBUILD downloads,
   `https://github.com/renanmt/exorchy/archive/refs/tags/v0.2.0.tar.gz`,
   exists as soon as the tag is pushed.
3. **AUR repo.** First time:
   `git clone ssh://aur@aur.archlinux.org/exorchy.git ~/aur/exorchy`
   (an empty repo: cloning a new name creates the package). Later:
   `git -C ~/aur/exorchy pull`.
4. **PKGBUILD.** Copy it in and fill in the checksum and metadata:

   ```bash
   cd ~/aur/exorchy
   cp ~/Projects/exorchy/packaging/aur/PKGBUILD .
   updpkgsums                              # replaces sha256sums=('SKIP')
   makepkg --printsrcinfo > .SRCINFO
   ```

   A new `pkgver` starts at `pkgrel=1`; a packaging-only fix raises `pkgrel`.
5. **Test.** Build exactly as users will:

   ```bash
   makepkg -si                             # builds, runs the tests, installs
   pkgctl build                            # clean chroot: catches missing deps
   namcap PKGBUILD *.pkg.tar.zst           # lint
   ```

   Launch eXorchy from the launcher (Super+Space) and check the icon, a
   download and a game start.
6. **Publish.**

   ```bash
   git add PKGBUILD .SRCINFO
   git commit -m "Update to 0.2.0"
   git push
   ```

   The package page is https://aur.archlinux.org/packages/exorchy and users
   install with `yay -S exorchy` (or `omarchy pkg aur add exorchy`).

## Notes

- `makepkg` warns "Package contains reference to $srcdir": the binary keeps
  its build path (`CARGO_MANIFEST_DIR`) for the source-checkout lookups. It
  only uses that path when the executable runs from inside the checkout, so a
  packaged binary never reads from an AUR helper's leftover build cache.
- The build downloads the Rust dependencies in `prepare()` (`cargo fetch
  --locked`, including the pinned librqbit fork from GitHub) and builds
  offline with `--frozen`.
- Keep `.SRCINFO` in step with the PKGBUILD: regenerate it on every change.
