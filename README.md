# PHAROS

**Persistent Host Access & Remote Operations Suite**

PHAROS is a modular Windows and macOS operations workspace built with Tauri,
native ES modules and Rust. The first implementation slice provides workspace selection,
master-password setup and a locally encrypted SQLCipher database.

## Architecture

- `web/app.js` — application shell and routing only.
- `web/modules` — isolated frontend feature modules.
- `src-tauri/src/modules` — isolated Rust domain modules and commands.
- `src-tauri/src/storage` — SQLCipher ownership and schema migrations.
- `src-tauri/src/platform` — Windows and macOS adapters.

Feature modules do not access another module's state or database tables
directly. Shared behavior is exposed through explicit commands and service
interfaces.

## Development

```bash
./scripts/run-dev.sh
```

On Windows, use:

```powershell
.\scripts\run-dev.ps1
```

The development launcher builds `pharos-agent` first and then starts the PHAROS
GUI. The Agent is a separate executable and remains alive when the GUI exits.
It exposes only an authenticated loopback IPC endpoint whose endpoint file is
stored in the current user's PHAROS runtime directory.

SSH host keys are stored in the encrypted workspace by host and port. PHAROS
marks first-seen keys as unknown and blocks silent replacement when a trusted
key changes. Active SSH transports send a keepalive every 20 seconds and are
closed after three unanswered keepalives.

The Agent can also be inspected directly during development:

```bash
src-tauri/target/debug/pharos-agent --start
src-tauri/target/debug/pharos-agent --status
src-tauri/target/debug/pharos-agent --stop
```

At workspace startup and after each Agent restart, PHAROS requests the KeePass
master password if a database is configured in Vaults. A wrong password reopens
the prompt with the error. API and deployment accounts for unfinished plans are
reloaded into Agent memory without starting a remote job. The master password
is held only in the unlocked GUI workspace memory, scoped to the Agent process
and KeePass database. Job starts and retries reuse that session without another
prompt. Locking or restarting the GUI clears it; an Agent restart requires a new
unlock. The Agent page shows its version and protocol alongside the GUI
requirements, flags older Agents, and offers **Stop**, **Kill Agent**, and **Start**.

A Queue job's **Restart** starts a new validation run from that job, preserves
completed predecessors, and queues subsequent steps again. Successful OLAM
validation opens a review when you click **Approve**: deployment settings,
validated targets and image versions, role variables, raw output, and payload.
Choose **Approve deployment** to launch that step, **Deny** to stop the Plan with
a denied status, or **Not now** to leave it awaiting approval. A changed validation
result must be reviewed again. **Restart Plan**
restarts all steps.

Both installer builds package the platform-native Agent executable beside the
PHAROS GUI. The macOS build produces a universal Agent binary; the Windows MSI
contains the x64 Agent binary.
The build scripts and installer workflow also open the finished DMG/MSI and
fail unless the packaged `pharos-agent` executable is present.

The initial frontend has no package-manager dependency. It uses native ES
modules and Tauri's global API, so a UI framework can be introduced later
without changing the Rust workspace contract.

## Installers

The GitHub Actions workflow in `.github/workflows/installers.yml` builds both
installers on native runners. Run **Build installers** manually from the
Actions tab to download them as workflow artifacts. Pushing a version tag such
as `v0.1.0` also creates a GitHub Release containing:

- a universal macOS `.dmg` for Apple Silicon and Intel Macs;
- a Windows x64 `.msi` installer.

For a local macOS build, install the Tauri CLI and run:

```bash
./scripts/build-macos.sh
```

For a local Windows build, run this from PowerShell on Windows:

```powershell
.\scripts\build-windows.ps1
```

Unsigned development installers can trigger operating-system warnings. Public
distribution should add Apple notarization and Windows code-signing secrets to
the workflow.

## Workspace files

A selected workspace directory contains:

- `pharos.sqlite` — SQLCipher-encrypted database.
- `pharos.sqlite.salt` — random salt used for master-key derivation.

The master password and derived database key are never written to disk.

