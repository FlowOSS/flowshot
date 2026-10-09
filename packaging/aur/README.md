# AUR packaging for FlowShot

AUR packages (owner decision 2026-10-09: ship `-git` and `-bin` only):

| Directory | AUR pkgbase | Role |
|---|---|---|
| `flowshot-git/` | `flowshot-git` | VCS package: builds the tip of `main` on the user's machine |
| `flowshot-bin/` | `flowshot-bin` | repackages the CI-built `flowoss-flowshot-<ver>-1-x86_64.pkg.tar.zst` release asset |

The recipe that BUILDS that release asset is **not an AUR package** and lives
outside this directory: [`packaging/arch/PKGBUILD`](../arch/PKGBUILD)
(pkgname `flowoss-flowshot`, consumed only by `.github/workflows/release.yml`
in a clean Arch container; kept ready so publishing a stable source package
later is a one-step aur.yml change).

`packaging/` files are 0BSD ([LICENSE](LICENSE)) per the Arch package-source
convention; the packaged software stays GPL-3.0-or-later.

## The name collision

The AUR pkgbase **`flowshot` belongs to an unrelated project** (an upload
wrapper for a Spectacle fork; it also installs `/usr/bin/flowshot`). It is
maintained, so there are no grounds to request it, and claiming
`provides=('flowshot')` would make `pacman -S flowshot` a choice between two
different programs. Rules baked into every PKGBUILD here:

- Never `provides=`/`conflicts=` the bare `flowshot` name.
- The usual `${pkgname%-git}` idiom is UNUSABLE for this family (`flowshot-git`
  minus `-git` is the other project's pkgbase) — the family name
  `flowoss-flowshot` is hardcoded.
- pacman's file-conflict check on `/usr/bin/flowshot` already prevents
  co-installation; the collision is documented, not declared.

## First publish (owner, once)

1. Create a **dedicated AUR account** for FlowShot (AUR auth is account-wide —
   a dedicated account keeps a leaked CI key from reaching anything else).
2. Register the pkgbases by pushing an initial commit for each: the AUR creates
   a pkgbase on first push (`ssh://aur@aur.archlinux.org/flowshot-git.git`,
   same for `flowshot-bin`).
3. Generate a CI keypair, add the public half to the AUR account's SSH keys,
   put the private half in the repo secret `AUR_SSH_PRIVATE_KEY`.
4. Host keys are PINNED in `publish.sh` (ed25519 fingerprint
   `SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4`, re-derived
   2026-10-09). If Arch ever rotates them, re-derive with
   `ssh-keygen -lf <(ssh-keyscan -t ed25519,ecdsa,rsa aur.archlinux.org)` and
   update both the known_hosts block and the fingerprint check.

## CI flows (`.github/workflows/aur.yml`)

- **flowshot-git**: republished only when files under
  `packaging/aur/flowshot-git/` change on main (a paths-filtered push job).
  Per-release bumps are deliberately absent: AUR guidelines forbid
  pkgver-only commits for VCS packages — `pkgver()` runs on the user's
  machine. The job rebuilds the package in an Arch container (makepkg +
  namcap) before publishing, so a broken PKGBUILD never reaches the AUR.
- **flowshot-bin**: published on `release: types [published]` — the workflow
  downloads the release's `.pkg.tar.zst` asset, computes its b2sum into the
  PKGBUILD, renders `.SRCINFO`, repackages once (no compilation), namcaps,
  and pushes. Asset name stability is a public contract (see the PKGBUILD's
  CI CONTRACT note).

Manual publish (same script the CI uses):

```sh
cd packaging/aur/flowshot-git
makepkg --printsrcinfo > .SRCINFO
../publish.sh flowshot-git . "Update flowshot-git"
```

## Build facts these PKGBUILDs encode (all empirically verified)

- `options=('!lto')` is REQUIRED: stock Arch `LTOFLAGS=-flto=auto` feeds slim
  LTO objects from the `cc`-built aws-lc-sys into an `ld.lld` link that
  rejects them (hundreds of `undefined symbol: aws_lc_0_45_0_*`). Reproduced
  in a clean container; zellij and lapce-git do the same. Rust-level LTO is
  unaffected and set via `CARGO_PROFILE_RELEASE_LTO=thin`.
- `clang` is a makedep (bindgen 0.72 in libspa-sys dlopens libclang.so,
  owned by `clang` on Arch); `cmake` is NOT (aws-lc-sys uses its cc builder).
- `checkdepends=('ttf-dejavu')` + `depends=('ttf-font')`: a fontless system
  panics cosmic-text in the editor text tests (reproduced; alacritty
  precedent).
- `wayland`, `vulkan-icd-loader`, `fontconfig`, `ttf-font` are dlopen/data
  deps invisible to ldd and namcap — namcap warns about them; the warnings
  are expected and the entries stay.
- `--all-features` must NEVER be used: it would compile the daemon's
  `test-drive` injection seam into user installs.
- `check()` is CI-viable headless: 41 suites / 1477 tests pass with no GPU,
  no Wayland, no session bus (GPU ui tests skip).
- The official `archlinux` docker image sets `NoExtract` for
  `usr/share/{man,doc,info}` — container validation must assert package
  CONTENTS (`pacman -Qpl`), never the installed filesystem.
