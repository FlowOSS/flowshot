# AUR packaging for FlowShot

| Directory | AUR pkgbase | Role |
|---|---|---|
| `flowshot-git/` | `flowshot-git` | VCS package: builds the tip of `main` on the user's machine |
| `flowshot-bin/` | `flowshot-bin` | repackages the CI-built `flowshot-<ver>-1-x86_64-Arch.pkg.tar.zst` release asset |

The recipe that BUILDS that release asset is **not an AUR package** and lives
outside this directory: [`packaging/arch/PKGBUILD`](../arch/PKGBUILD)
(pkgname `flowshot`, consumed only by the release workflow's `package-arch`
job in a clean Arch container).

`packaging/` metadata files are 0BSD ([LICENSE](LICENSE)) per the Arch
package-source convention; the packaged software stays GPL-3.0-or-later.

## Naming

The software's package name is **`flowshot`** — the built packages (release
asset and anything derived from it) install as `flowshot`, and every family
member carries `provides=("flowshot=$pkgver")` + `conflicts=('flowshot')`,
the standard takeover pattern: installing FlowShot replaces anything else
claiming the name. The AUR **pkgbase** `flowshot` happens to be squatted by
an unrelated install script; that only constrains AUR *submission* names, so
the submissions use the `-git` / `-bin` suffixes (the latter required by AUR
rules for prebuilt packages anyway). If/when FlowShot reaches the official
repos, the name is concrete.

## First publish (once, manual)

1. Create a **dedicated AUR account** for FlowShot. AUR authentication is
   account-wide (there are no per-package deploy keys), so a dedicated account
   keeps CI credentials away from any personal packages.
2. Generate a CI keypair; add the public half to the account's SSH keys; put
   the private half in the repo secret `AUR_SSH_PRIVATE_KEY`. Both pkgbases
   are created automatically on first push.
3. Host keys are PINNED in `publish.sh` (ed25519 fingerprint
   `SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`, verified 2026-10-09).
   If Arch rotates keys, re-derive with
   `ssh-keygen -lf <(ssh-keyscan -t ed25519,ecdsa,rsa aur.archlinux.org)` and
   update both the known_hosts block and the fingerprint check.

Until the secret exists, the publish workflow runs its build gate and skips
publishing with a notice — the first AUR release is made manually:

```sh
cd packaging/aur/flowshot-git
makepkg --printsrcinfo > .SRCINFO
../publish.sh flowshot-git . "Initial commit"
```

## CI flows (`.github/workflows/aur.yml`)

- **flowshot-git**: republished only when files under
  `packaging/aur/flowshot-git/` change on main. Per-release bumps are
  deliberately absent — AUR guidelines forbid pkgver-only commits for VCS
  packages (their `pkgver()` runs on the user's machine). The job rebuilds
  the package in an Arch container (full makepkg incl. `check()`, then
  namcap) before publishing, so a broken PKGBUILD never reaches the AUR.
- **flowshot-bin**: published on `release: types [published]` — the workflow
  downloads the release's `.pkg.tar.zst`, writes its b2sum into the PKGBUILD,
  renders `.SRCINFO`, repackages once (no compilation), namcaps, and pushes.
  Asset name stability is a public contract: the asset name embeds
pkgver/pkgrel plus the `-Arch` marker, and the publish workflow rewrites
pkgver + b2sums per release — renaming the asset without updating the
`-bin` source URL breaks every user build.

Publishing is hand-rolled (`publish.sh`: pinned host keys, staged-diff no-op
guard, master-only push, SKIP-checksum refusal) rather than a third-party
GitHub Action: the widely used ones either run `makepkg --nodeps` as their
"test" (which cannot build FlowShot — no clang/pipewire/librsvg in the
image), run unpinned third-party Docker images inside the workflow that
holds the AUR SSH key, or rewrite `.SRCINFO` textually instead of
regenerating it.

## Build facts these PKGBUILDs encode

- `options=('!lto')` is required: stock Arch `LTOFLAGS=-flto=auto` feeds slim
  LTO objects from the cc-built aws-lc-sys into an `ld.lld` link that rejects
  them (hundreds of `undefined symbol: aws_lc_0_45_0_*`). Rust-level LTO is
  unaffected and set via `CARGO_PROFILE_RELEASE_*` in `build()`.
- `clang` is a makedep (bindgen in libspa-sys dlopens libclang.so, owned by
  `clang` on Arch); `cmake` is not needed (aws-lc-sys uses its cc builder on
  x86_64-linux).
- `checkdepends=('ttf-dejavu')` + `depends=('ttf-font')`: a fontless system
  panics cosmic-text in the editor text tests and at runtime.
- `wayland`, `vulkan-icd-loader`, `fontconfig`, `ttf-font` are dlopen/data
  dependencies invisible to ldd and namcap — the namcap "may not be needed"
  warnings for them are expected; the entries stay.
- `--all-features` must never be used: it compiles the daemon's `test-drive`
  injection seam into user installs.
- `check()` passes headless (no GPU, Wayland, or session bus needed): GPU
  tests skip, the rest run normally.
- `flowshot-bin` carries no `.install` hook: pacman's own
  `gtk-update-icon-cache` and `update-desktop-database` hooks fire on the
  installed file paths; `options=('!strip' '!lto' '!debug')` because the
  asset is already a compiled Arch package and must not be re-processed.
- The official `archlinux` Docker image sets `NoExtract` for
  `usr/share/{man,doc,info}` — container validation must assert package
  CONTENTS (`pacman -Qpl`), never the installed filesystem.
