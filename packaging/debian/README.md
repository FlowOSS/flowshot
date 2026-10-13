# Debian & Ubuntu Packaging for FlowShot

FlowShot supports two packaging paths for Debian and Ubuntu users:

1. **Path A (Current): Standalone `.deb` Releases via `cargo-deb`**  
   Pre-built `.deb` binary packages attached to GitHub Releases for direct installation.
2. **Path B (Roadmap): Ubuntu Launchpad PPA & Debian Archive**  
   APT repository hosting allowing installation via `add-apt-repository` and `apt install`.

---

## Path A: Standalone `.deb` Release Packages

FlowShot uses `cargo-deb` to package production binaries, desktop entries, icons, man pages, and shell completions into a standard Debian package.

### Configuration (`flowshot-cli/Cargo.toml`)

Debian package metadata is configured under `[package.metadata.deb]` in `crates/flowshot-cli/Cargo.toml`:
- **Package Name:** `flowshot`
- **Section:** `graphics`
- **Priority:** `optional`
- **Architecture:** `amd64` (`x86_64`)
- **Dependencies (`Depends`):** `$auto`, `libpipewire-0.3-0`, `hicolor-icon-theme`
- **Recommends:** `xdg-desktop-portal`

`$auto` instructs `cargo-deb` to inspect the compiled ELF binaries using `objdump` and automatically resolve the exact Debian library versions required by the system:
- `libc6 (>= 2.35)`
- `libgcc-s1`
- `libfontconfig1`
- `libx11-6`, `libxcb1`, `libxcb-randr0`, `libxcb-shm0`, `libxcb-xfixes0`

### Building Locally

To build the `.deb` package on an Ubuntu or Debian system (or inside an Ubuntu container):

```bash
# 1. Install prerequisites
sudo apt-get install -y cargo clang librsvg2-bin libpipewire-0.3-dev

# 2. Install cargo-deb
cargo install cargo-deb

# 3. Build release binaries
cargo build --release

# 4. Run the packaging script
./packaging/debian/build-deb.sh
```

The resulting package is written to `target/debian/flowshot_<version>_amd64.deb`.

### Installation by Users

Users can install the package directly:

```bash
sudo apt install ./flowshot_<version>_amd64.deb
```

`apt` will automatically resolve and install any missing system dependencies.

---

## Path B: Ubuntu Launchpad PPA & Official Debian Archive

To enable `sudo apt install flowshot` without manual file downloads, FlowShot can be published to an Ubuntu PPA on Launchpad.

### Architecture Requirements for Launchpad

Launchpad does not accept compiled `.deb` files; it requires **source packages** (`.dsc`, `.tar.xz`, `_source.changes`) and compiles the binaries on Canonical's build farm.

To support Launchpad, a `debian/` directory is needed:

```text
debian/
├── control          # Package source and binary definitions, build-depends
├── rules            # Build script executed by dpkg-buildpackage (uses dh-cargo)
├── changelog        # Debian-formatted version history
├── copyright        # DEP-5 machine-readable copyright file
└── source/
    └── format       # "3.0 (quilt)"
```

### Steps to Establish an Official PPA

1. **Register on Launchpad**:
   - Create the PPA: `ppa:flowoss/flowshot` on [Launchpad.net](https://launchpad.net).
   - Add maintainer GPG signing key to the Launchpad profile.
2. **Build Dependencies (`debian/control`)**:
   - `debhelper-compat (= 13)`
   - `cargo`, `rustc`
   - `clang`, `pkg-config`
   - `libpipewire-0.3-dev`
   - `librsvg2-bin`
3. **Automate PPA Uploads**:
   - On release tags, GitHub Actions prepares the source tarball, signs the `.changes` file with the maintainer's GPG key, and uploads it to Launchpad via `dput`:
     ```bash
     debuild -S -sa
     dput ppa:flowoss/flowshot flowshot_<version>_source.changes
     ```
   - Launchpad builds binaries for all active Ubuntu LTS releases (Ubuntu 24.04 Noble, Ubuntu 22.04 Jammy).
