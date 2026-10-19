# FlowShot Windows Packaging

This directory contains the packaging specifications and branding assets for native Windows distribution.

## Contents

- `flowshot.ico`: Multi-layer Windows application icon (16x16, 24x24, 32x32, 48x48, 64x64, 128x128, 256x256) rasterized from `assets/logo.png`.
- `flowshot.iss`: Inno Setup script that generates `flowshot-setup-<version>-x86_64.exe`.

## Installer Features

- **Installation Directory**: Custom directory selection, defaulting to `{autopf}\FlowShot` (`C:\Program Files\FlowShot` or user local app data when non-elevated).
- **Shortcuts**: Start Menu shortcut for `FlowShot` with our icon, and an optional Desktop shortcut.
- **Startup Integration**: Optional "Launch FlowShot when Windows starts" task that writes to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- **Uninstallation**: Registered uninstaller in Windows *Installed Apps* / *Add or Remove Programs* with the FlowShot icon, version, publisher, and automatic daemon shutdown prior to file removal.

## Building the Installer Locally (Windows)

1. Build release binaries:
   ```cmd
   cargo build --release -p flowshot-cli -p flowshot-daemon
   ```
2. Compile with Inno Setup:
   ```cmd
   iscc /DAppVersion=0.4.3 packaging/windows/flowshot.iss
   ```
3. The setup executable will be generated at:
   ```
   target/windows-dist/flowshot-setup-0.4.3-x86_64.exe
   ```
