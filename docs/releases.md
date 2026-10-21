# Releases and administrator-free updates

## Development and local Mac release commands

Run `./run-dev.sh` from the repository root to build and start the debug UI.
The running Agent is reused.

Run `./run-release.sh` to choose a release version, run checks, build a native
Developer ID signed Mac DMG and publish it to `GateTree-Ops-updates`.
No switches are needed: press Enter to accept the offered version, or type another.
Authenticate once using `gh auth login`. The default version is the next patch
after both the checkout version and existing local packages.
An empty updates repository is initialized with a binary-download README before
the first release is uploaded or published; GitHub needs a commit for release tags.

```sh
./run-release.sh
```

`--upload-only` resumes an existing local package without rebuilding.
Publishing is the default; `--draft` uploads for review instead. Published versions cannot be
overwritten. Only the DMG, SHA-256 checksum, signed updater archive, build metadata,
release notes and `latest.json` are uploaded. This command does not commit or push
source or change the debug version. These local packages are not notarized.
Publishing activates automatic updates for the CPU architecture built on this
Mac; the other Mac architecture and Windows are not advertised. For a notarized
universal automatic-update release, use `scripts/release-macos.sh` below.

GateTree-Ops uses its own Tauri update signing key and public binary feed:
`https://github.com/zsoltkarman/GateTree-Ops-updates/releases/latest/download/latest.json`.
The old GateTree Sparkle feed, identity and signing key remain separate.

## User experience

Installed release builds check eight seconds after launch and show an update
only when one is available. **Help → Check for Updates** also checks manually.
The dialog shows the version and release notes, with **Later** and **Update and
restart**. Downloads and signatures are checked before application replacement.
Offline automatic checks are quiet; manual checks show errors.

The installation directory must be writable by the current user. The app checks
actual write access; a writable `/Applications` installation is supported on
macOS. If you do not have write access, use a per-user installation:

- macOS: open the DMG, create `~/Applications` if necessary, and copy
  `GateTree-Ops.app` there. Finder → Go → Go to Folder → `~/Applications`.
  Run the copied app, not the copy still inside the mounted DMG. Do not drag to
  the DMG's system `/Applications` shortcut when you lack write access there.
- Windows: use the **Setup EXE**, configured as an NSIS current-user installation.
  It installs under the user's local application directory. The MSI remains
  available for managed installations; it is not the administrator-free update
  path. Existing machine-wide/MSI installations should move to the per-user EXE.
  WebView2 must be available; corporate policies or missing prerequisites can
  still require IT support. Windows updater signatures are independent of
  optional Authenticode signing and SmartScreen reputation.

The Agent refuses installation while deployments are running or awaiting approval,
or SSH sessions are open. Close the dialog, finish/stop those jobs and sessions,
and retry. An idle Agent is reserved before download, blocking new jobs from
other GUI instances as well. The reservation is cancelled on a failed download
or backup, and expires after ten minutes if the GUI crashes. The Agent stops
immediately before replacement and the new GUI uses the newly bundled Agent.
Both have the same version and protocol. A failed installation attempts to
restore the previous Agent. Debug builds never replace themselves or check the
public feed automatically; manual checks explain that updates require a release.

## Encrypted backups

Before installation, an unlocked workspace receives a consistent online SQLCipher
backup including committed WAL data, its salt, and persisted Agent job state.
No master password, derived key or KeePass password is written to this backup.
Backups are stored in the workspace's `.gatetree-ops-backups/before-update-*`.

On first open after a schema change, a separate encrypted `before-migration-*`
backup is made before migrations. This also protects workspaces that were locked
when updating, or were not open on update day. A database from a newer schema is
rejected rather than opened by older code. Keep the original master password;
restore a matching database/salt pair from a backup with the application closed.
A backup failure prevents installation/migration. Backups are not automatically
deleted; users control their retention. There is no automatic database downgrade.

## One-time release setup

1. Authenticate GitHub CLI with an account that can manage both repositories:
   `gh auth login`.
2. Run `./scripts/configure-release.sh`. It creates the public **binary-only**
   `zsoltkarman/GateTree-Ops-updates` repository if needed, and uploads the local
   updater key to the private Actions secret `TAURI_SIGNING_PRIVATE_KEY` in
   `zsoltkarman/GateTree-Ops`.
3. Add the following Actions secrets in the **source** repository:

| Secret | Purpose |
| --- | --- |
| `GATETREE_OPS_UPDATES_TOKEN` | A token scoped to the updates repository with contents write access; creates draft releases and uploads binaries. |
| `TAURI_SIGNING_PRIVATE_KEY` | Set by the setup script. The matching public key is already embedded in the application. |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Only if using a password-protected updater key. The initial local key is protected by filesystem permissions and uses an empty password. |
| `APPLE_CERTIFICATE` | Base64-encoded Developer ID Application certificate and private key exported as a `.p12`. |
| `APPLE_CERTIFICATE_PASSWORD` | Password protecting that `.p12`. |
| `APPLE_SIGNING_IDENTITY` | Full Developer ID Application signing identity. |
| `APPLE_API_KEY_ID` | App Store Connect notarization API key ID. |
| `APPLE_API_ISSUER_ID` | Notarization API issuer ID. |
| `APPLE_API_PRIVATE_KEY` | Contents of the notarization `.p8` key. |

The existing GateTree Developer ID and notarization credentials may also sign
GateTree-Ops; the **Tauri updater key is separate**. Never commit secrets or put a
GitHub download token into the shipped client. The update repository must be
public so coworkers can download without GitHub credentials. Only binaries,
signatures, notes and `latest.json` are released there; workspaces, accounts,
connection exports and source files are not uploaded.

The newly generated local private updater key is:
`~/.config/gatetree-ops/updater.key` (owner-only permissions).
Its public companion is `updater.key.pub`. Back up the private key securely;
future releases must use the same key. The setup script does not overwrite it.

## Create a release manually

1. Commit/push the reviewed source and workflow to `GateTree-Ops`.
2. Actions → **Build release** → **Run workflow**.
3. Enter a stable version such as `0.1.15` and the release notes.
4. macOS and Windows runners run checks and build the GUI plus native Agent.
   The macOS app is Developer ID signed and notarized. Both updater packages
   carry the dedicated updater signature.
5. A **draft** is created in `GateTree-Ops-updates` containing the DMG, per-user
   Setup EXE, MSI, signed updater packages, and `latest.json`.
6. Test the installers and update from the previous installed release, review
   the notes, then publish that draft. Nothing is offered to coworkers while
   it remains a draft. A failed platform build produces no draft release.

A `v*` tag in the source repository also starts the workflow, using
`RELEASE_NOTES.md`. The manual input version is applied consistently to GUI,
Agent, Agent core, Tauri config and Cargo lockfile within the build; it does not
commit back to the source repository. Use `python3 scripts/set-version.py 0.1.16`
locally when preparing the next source version.

Do not reuse or overwrite an existing release version. A published version's
update signatures and archives must stay immutable. Removing/withdrawing a bad
release prevents new users from receiving it; ship a higher corrected version
for existing users. A draft is the final review point before distribution.

## Local builds

For a local native-architecture DMG matching the old GateTree packaging flow:

```sh
./scripts/package-macos-local.sh 0.1.16
```

This stages the source separately, builds the GUI and Agent, signs both the app
and DMG using the available Developer ID identity, and verifies the bundled
Agent. Output is under `dist/local/0.1.16/`, including a signed updater archive
and native-CPU `latest.json`. It does not notarize or upload. macOS may require manual approval to open the local
package. Use the notarized release workflow below for distribution to coworkers.

Docker on this Mac runs Linux containers. The supported release path builds the
DMG on macOS and the MSI on Windows (a native host, VM or Actions runner).
Tauri supports cross-compiling a Windows NSIS Setup EXE from Linux/macOS with
additional tooling, but that does not produce the MSI or a macOS DMG. The
existing Windows script builds and verifies both installers on Windows.
See the [Tauri Windows cross-compilation documentation](https://v2.tauri.app/distribute/windows-installer/#build-windows-apps-on-linux-and-macos).

The same scripts work manually on native build hosts with Rust, Tauri CLI v2 and
platform build prerequisites. Set `TAURI_SIGNING_PRIVATE_KEY` to the key file or
its content (defaults to the local path above). For macOS also configure the
Developer ID and Tauri notarization environment, then run:

```sh
./scripts/build-macos.sh
```

On Windows:

```powershell
./scripts/build-windows.ps1
```

Collect updater archives/installers **and their `.sig` files** in one directory.
Create the feed with:

```sh
python3 scripts/create-update-manifest.py release-artifacts --version 0.1.15 --notes-file RELEASE_NOTES.md
```

The manifest requires both the universal Mac archive and the Windows NSIS EXE.
It points Apple Silicon and Intel Macs to the same universal signed archive, and
Windows x64 to NSIS. A local Windows build does not need the Mac's private key
copied into the source tree; supply the shared signing key through a secure local
file or environment. Upload the resulting assets together to a draft release;
publishing it is a separate action.

The first updater-enabled release must be installed manually once. Earlier
GateTree-Ops/PHAROS builds have no updater and cannot discover it themselves.

References: [Tauri updater](https://v2.tauri.app/plugin/updater/),
[Windows installers](https://v2.tauri.app/distribute/windows-installer/),
[macOS signing/notarization](https://v2.tauri.app/distribute/sign/macos/).

## One-command release from your Mac

The local workflow builds from a separate source copy under
`~/Library/Caches/GateTree-Ops/release-build`, including your current uncommitted
source changes. It records their digest and Git commit in `build-info.json`.
It does not commit/push source, change the checkout version, replace debug
binaries, or restart the running Agent. Only the DMG, signed updater archive,
manifest and build metadata are uploaded to the public updates repository.

One-time prerequisites:

- `gh auth login` with write access to `GateTree-Ops-updates`.
- Rustup with a working Rust toolchain. The build installs the Apple Silicon
  and Intel targets. Homebrew Rust alone currently lacks the Intel target on
  this Mac; install/configure Rustup before attempting a universal release.
  A separate local release toolchain can be selected with `CARGO_HOME` and
  `RUSTUP_HOME` in `release.env`, without changing the debug shell's Rust setup.
- Tauri CLI v2 and the local updater key described above.
- The Developer ID certificate must have an accessible private key in Keychain.
  The script selects it automatically when exactly one Developer ID identity exists.
- Configure Apple notarization credentials in a private shell environment file
  `~/.config/gatetree-ops/release.env` (`chmod 600`). For API-key authentication:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: Zsolt Karman (PZ6YC7GM6J)"
APPLE_API_KEY="YOUR_KEY_ID"
APPLE_API_ISSUER="YOUR_ISSUER_ID"
APPLE_API_KEY_PATH="/absolute/path/to/AuthKey.p8"
```

Alternatively supply `APPLE_ID`, `APPLE_PASSWORD` (an app-specific password),
and `APPLE_TEAM_ID`. Keep this config and keys outside the source repository.
These credentials stay local; this workflow does not need GitHub Actions secrets.

From the source directory:

```sh
./scripts/release-macos.sh 0.1.16 --check
./scripts/release-macos.sh 0.1.16
```

The second command runs checks, builds and verifies the signed/notarized universal
app and DMG, then uploads a **draft** release. Local output is in
`dist/releases/0.1.16/`. Edit `RELEASE_NOTES.md` first, or pass `--notes-file PATH`.
Use a version higher than the last published version.

To build and publish in one operation, explicitly include `--publish`:

```sh
./scripts/release-macos.sh 0.1.16 --publish
```

To publish the already uploaded/tested draft using the same prepared assets:

```sh
./scripts/release-macos.sh 0.1.16 --upload-only --publish
```

`--build-only` creates local installers without GitHub authentication.
`--upload-only` can recover a partially uploaded draft; published releases cannot
be overwritten. A failed build never starts an upload.

This command produces a **Mac-only** feed for Intel and Apple Silicon. It never
labels an older Windows binary as the new version. For a release containing both
platforms, use the Actions workflow above or collect the matching signed Windows
artifacts before creating the default combined manifest. Windows has no new
update package in a Mac-only release.
