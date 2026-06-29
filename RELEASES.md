# Releases

TARDIS releases are cut from annotated tags (convention: `tardis-vX.Y.Z`; legacy `vX.Y.Z` is accepted by [`.github/workflows/release.yml`](.github/workflows/release.yml) for compatibility with the pre-existing `v0.1.0` tag). Each release publishes a `SHA256SUMS.txt` alongside the per-platform bundles so end-users can verify a download before installing.

> **Posture for first-cut releases.** Bundles are **not** code-signed. macOS Gatekeeper will require `Right-click → Open` on the first launch; Windows SmartScreen will warn about an unknown publisher; Linux GPG sidecars are not produced. Real code signing + notarisation is a separate task (the **Code signing + notarisation** checkbox in the [`## Generating a release`](README.md#generating-a-release) section of `README.md`).

> **Filename note.** Tauri uses the `productName` from `src-tauri/tauri.conf.json` verbatim in installer filenames — and `productName` is `"TARDIS v1"` (with a literal space). The space shows up in the on-disk filename (`TARDIS v1_0.2.0_amd64.deb`) but must be URL-encoded as `%20` in `curl` commands (`TARDIS%20v1_0.2.0_amd64.deb`).

## Download

- **GitHub Releases:** <https://github.com/godie/tardis/releases> — pick the latest tag (e.g. `tardis-v0.2.0`) and download the artefact for your platform.
- **Verify integrity** before installing (note the `%20` URL-encoding on the space in `TARDIS v1`):

  ```bash
  TAG=tardis-v<version>     # e.g. tardis-v0.2.0
  BASE="https://github.com/godie/tardis/releases/download/${TAG}"
  curl -L -O "${BASE}/SHA256SUMS.txt"
  curl -L -O "${BASE}/TARDIS%20v1_<version>_amd64.deb"
  sha256sum -c --ignore-missing SHA256SUMS.txt
  ```

## Install (Debian / Ubuntu)

1. Download the `.deb` for your architecture (`amd64` is the common case):

   ```bash
   curl -L -O https://github.com/godie/tardis/releases/latest/download/TARDIS%20v1_<version>_amd64.deb
   ```

2. Install via `apt` (resolves runtime system deps like `libwebkit2gtk-4.1-dev` through your package manager):

   ```bash
   sudo apt install ./'TARDIS v1_<version>_amd64.deb'
   ```

3. Launch from your application launcher, or run `tardis --help` from a terminal (the binary lands at `/usr/bin/tardis` after install).

A portable `.AppImage` is also shipped for distributions that prefer not to install packages system-wide:

```bash
curl -L -O https://github.com/godie/tardis/releases/latest/download/TARDIS%20v1_<version>_amd64.AppImage
chmod +x 'TARDIS v1_<version>_amd64.AppImage'
./'TARDIS v1_<version>_amd64.AppImage'
```

`.AppImage` is **unsigned by convention** in this first cut; the README's `Code signing + notarisation` item will revisit the GPG sidecar when project-wide signing lands.

## Install (macOS)

1. Download the `.dmg` for your architecture (`x64` = Intel, `aarch64` = Apple Silicon):

   ```bash
   curl -L -O https://github.com/godie/tardis/releases/latest/download/TARDIS%20v1_<version>_<arch>.dmg
   ```

2. Open the `.dmg` and drag **TARDIS v1** to `/Applications`.

3. First launch: because the bundle is **not** signed with a Developer ID yet, right-click the app in `/Applications` → **Open** → confirm the dialog. Subsequent launches can be normal double-clicks.

For a Homebrew Cask distribution (not yet wired — projected in the README release section):

```bash
brew install --cask tardis   # projected, not yet a tap formula
```

## Install (Windows)

1. Download the `.msi` (`x64_en-US` is the common case) from the GitHub release:

   ```
   https://github.com/godie/tardis/releases/latest/download/TARDIS%20v1_<version>_x64_en-US.msi
   ```

2. Double-click the `.msi` and click through the WiX installer (Next → Next → Finish). `tardis` lands at `%LOCALAPPDATA%\Programs\TARDIS v1\` and on the Start Menu.

For `winget` distribution (not yet wired):

```powershell
winget install tardis       # projected, not yet a winget package
```

A `.exe` (NSIS) installer is also uploaded alongside the `.msi`; both produce the same install location.

## Update

TARDIS does **not** ship an in-app updater yet — re-run the install walkthrough for the new tag. A future release may add Tauri 2's built-in `tauri-plugin-updater` (referenced from the `Next:` items in [`README.md` § Tauri UI Shell](README.md#tauri-ui-shell)).

## Cross-references

- Release workflow source — [`.github/workflows/release.yml`](.github/workflows/release.yml)
- Release item in `## Generating a release` of [`README.md`](README.md)
- Operational roadmap — [`ROADMAP.md`](ROADMAP.md)
