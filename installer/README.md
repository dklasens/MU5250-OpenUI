# Open U60 Pro desktop installer

A lightweight Tauri application for installing, repairing and updating the Rust
agent and web dashboard on a ZTE U60 Pro (MU5250). No terminal, Python, Node.js,
Rust toolchain or separately installed ADB is needed by end users.

## Downloads and requirements

Download from [GitHub Releases](https://github.com/dklasens/MU5250-OpenUI/releases/latest).

| Computer | Recommended download | Alternatives |
|---|---|---|
| macOS 15+ · Apple Silicon | `aarch64.dmg` | `aarch64.app.zip` |
| macOS 15+ · Intel | `x64.dmg` | `x86_64.app.zip` |
| Windows x64 | `x64-setup.exe` | MSI for managed installation; portable ZIP |

**v2.5 signing:** Mac packages are ad-hoc signed and not Apple-notarized. Windows
packages are unsigned. The operating system may show an unverified-publisher
warning. Download only from the linked project release and verify its checksums.

Windows requires a compatible modem USB driver and OpenSSH Client (Windows
Settings → Optional features). Setup installs WebView2 when required; this may
need internet access. The portable ZIP requires an existing WebView2 runtime
(version 105 or newer) and must be extracted with its `platform-tools` directory.
Neither package installs a modem USB driver automatically.

Both Mac architectures are built and tested on macOS 15 runners. Windows checks
run on GitHub's Windows runner; they do not establish compatibility with every
Windows client configuration or USB driver.

## Connect → Check → Install → Open dashboard

1. Join the modem Wi-Fi or connect through USB tethering. For unlocking or ADB
   installation, also connect a USB data cable. Keep this computer connected to
   the modem network throughout installation.
2. Detect the modem. If more than one compatible modem is attached, select it.
3. For a locked device, enter the router admin password and backup-key suffix.
   The key is available in the
   [upstream issue #8 discussion](https://github.com/jesther-ai/open-u60-pro/issues/8).
   The installer does not embed the suffix.
4. Choose a dashboard password for a new installation. Updates and repairs
   default to **Keep existing password** and **Keep existing PIN setting**.
   Password changes, setting a six-digit PIN and removing a PIN are separate,
   explicit choices.
5. Select **Check device**. The installer checks host prerequisites, downloads
   or reuses verified files and verifies the device. Review the model, firmware
   and software version. Editing settings requires another check.
6. Select **Install**, **Repair** or **Update**. Locked devices receive a separate
   confirmation after the actual backup and downloads have been prepared and
   verified, immediately before upload/restore.
7. Wait for verification, then select **Open dashboard**. Connection details can
   also be copied. The dashboard password is separate from the stock router
   admin password.

A locked-device check uses the normal temporary configuration export and prepares
an unlock backup locally; it does not upload or restore it. Storage and shell
checks become available after unlocking. This limitation is not a guarantee of
compatibility with every firmware. Read [SAFETY.md](../docs/SAFETY.md) first.

## Safe stopping and recovery

**Stop after current check** is available during preparation. It stops at a safe
boundary, so an in-flight bounded download or check may finish first. Once device
changes begin, normal window close and application quit are blocked until
verification or recovery finishes. Force termination, power loss and unplugging
cannot be prevented by the application.

Detection finds pending on-device deployment transactions after reopening. It
blocks a new installation and offers to restore the previous installation, or
clear an incomplete preparation where the recovery records prove that no live
files were activated. Recovery rechecks the device identity and recovery helper;
ambiguous or incompatible records require manual inspection. A failed deployment
also attempts recovery automatically.

Retain recovery snapshots until the installation has been checked. The installer
does not automatically prune recovery data. An interrupted unlock is handled by
the stock backup/restore mechanism; the deployment snapshot covers subsequent
agent/dashboard installation, not a firmware-level restore.

## Offline files and SSH identity

Verified bundles are cached automatically by content. Later checks reuse a
matching valid bundle, including when online release selection is unavailable.
Use **Offline bundle and diagnostics → Browse** to choose a separate bundle.
Files are checked again before use. Installer and payload major/minor versions
must match; an incompatible future release requires its corresponding installer.
The checked release is pinned for the following installation.

The installer owns a dedicated SSH key in the operating system's local application
data directory under `open-u60-pro/ssh/id_ed25519`. It does not replace the user's
personal SSH key. Existing default keys can be used to connect to an older
installation for migration; encrypted keys must already be available through the
SSH agent, otherwise use ADB. The newly installed dedicated key is verified
explicitly. Keep a private backup of that identity if SSH maintenance is needed
from a replacement computer. Copy connection details for the matching SSH command.

Original encrypted modem backups are retained under `open-u60-pro/recovery` in
local application data. Temporary decrypted files are removed on normal success
or failure. **Keep temporary files** is an explicit diagnostic option and may
retain sensitive modem configuration. Never publish these files or SSH keys.

## Implementation and validation

- React UI with a constrained Tauri command interface, native folder picker,
  keyboard-contained dialogs and single-instance handling.
- Bounded native subprocess execution, immutable request validation and
  authenticated device identity checks.
- Pinned Google ADB 37.0.1: only ADB, required libraries and notices are bundled.
  The cache stages files atomically and validates their hashes before execution.
- Staged agent/dashboard activation, startup-script syntax checks, rollback
  snapshots and key-only SSH verification.
- Native unit tests, UI workflow tests, package startup diagnostics and Windows
  setup/uninstall checks in GitHub Actions.

Package startup diagnostics launch the real WebView and execute `adb version`.
They disable modem commands and do not perform physical deployment. UI tests and
published wireframes use sample data. The physical v2.3 deployment evidence does
not substitute for a v2.5 installer end-to-end hardware test. Dashboard/API traffic remains
HTTP on the LAN.

## Build and test

Developer prerequisites: Node.js `^20.19.0 || >=22.12.0`, Rust and the Tauri
platform build prerequisites. From the repository root:

```sh
python3 scripts/prepare-installer-tools.py darwin  # use windows on Windows
npm ci --prefix installer
npm run build --prefix installer
npm test --prefix installer
cargo test -p mu5250-installer -p process-runner --locked
cargo clippy -p mu5250-installer -p process-runner --all-targets --locked -- -D warnings
npm run tauri --prefix installer -- build
```

A native executable launched with `--startup-check <report.json>` performs the
package startup diagnostic and exits; modem commands are disabled in that mode.

The manual installer workflow checks out its explicit `release_tag` for both
checks and builds. A blank tag produces build-only artifacts. Tagged production
releases include checksums for every published download.

## Optional release signing

Unsigned/ad-hoc builds remain supported and must be labelled in release notes.
To enable Apple Developer ID signing and notarization, configure GitHub secrets
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `KEYCHAIN_PASSWORD`, `APPLE_ID`,
`APPLE_PASSWORD` (app-specific password), and `APPLE_TEAM_ID`. Configured Developer
ID builds require complete notarization credentials and pass Gatekeeper checks.

For certificate-based Windows signing, configure `WINDOWS_CERTIFICATE` (base64
PFX) and `WINDOWS_CERTIFICATE_PASSWORD`. The workflow imports it temporarily,
configures SHA-256 Authenticode signing with timestamping, verifies the setup
signature and removes temporary credentials. Certificates requiring a remote
signing service or hardware token need that provider's integration separately.
Do not put certificates or passwords in source or issue comments.

## Wireframes

[View the v2.5 installer wireframes](../docs/INSTALLER-WIREFRAMES.md). They render
the actual React interface through a separate sample-only bridge, which is not
included in the desktop bundle.

To regenerate, run `npm run wireframes --prefix installer` in one terminal, then
`npm run capture:wireframes --prefix installer` in another. Install Playwright's
Chromium with `npx --prefix installer playwright install chromium` first.
