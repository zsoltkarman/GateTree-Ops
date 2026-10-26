# GateTree-Ops

**Remote access and deployment operations**

GateTree-Ops is a modular Windows and macOS operations workspace built with Tauri,
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
./run-dev.sh
```

On Windows, use:

```powershell
.\scripts\run-dev.ps1
```

For a local macOS build and GitHub package upload, run `./run-release.sh`.
It commits non-ignored source changes and pushes the current branch to `origin`
before building, so the release source (including new files) is available in Git.
It offers the next version, builds and publishes the DMG and automatic update
to `GateTree-Ops-updates`. No switches are needed. See [release instructions](docs/releases.md).

The development launcher builds `gatetree-ops-agent` first and then starts the GateTree-Ops
GUI. The Agent is a separate executable and remains alive when the GUI exits.
It exposes only an authenticated loopback IPC endpoint whose endpoint file is
stored in the current user's GateTree-Ops runtime directory.

The header Quick Launcher stores pinned connections in the encrypted workspace.
Right-click a connection and choose **Add to Quick Launcher**, or drag it onto
the header strip. Drag launcher buttons to reorder them; their context menu
provides a separate label, left/right moves, and removal. Clear the label to
follow the source connection name. A **»** menu holds connections that do not fit.
URL buttons open the existing URL action; SSH buttons use the existing connection
and host-key checks. Other connection types open their existing detail view.

The workspace tab strip below the header has a permanent **Overview** with
sessions, deployment Plans and external-browser activity. Open SSH sessions and
selected Plans have individual tabs with status indicators. Drag tabs to reorder
them; the right-hand dropdown lists every open tab. Switching tabs preserves the
terminal DOM, scrollback and Plan view. Hidden SSH views pause reading while the
Agent retains their output. Closing an SSH tab disconnects that session; closing
a Plan tab only closes the view, and execution continues in the Agent. **Detach**
moves an SSH view to its own window. Live sessions are available again after a
GUI restart. URL links remain in the system browser and Overview.

Inactive Plans can be deleted directly beside **Focus** in the Agent activity
list. Confirming deletion removes the Plan and its job history from both the
Queue and Agent, including after restart. Stop active Plans before deleting them.

New SSH connections prompt for a username in an empty field, including local
OpenSSH connections. OpenSSH receives that explicit username and never falls
back to the local operating-system account. **Close** disconnects the SSH session
and closes the terminal; **Detach** keeps the session open in a separate window.

New SSH connections default to **Terminal login**, which opens the terminal
before authentication so password, MFA and SSH configuration prompts are visible
and interactive from the start. For an existing connection, choose **Log in in
terminal** in its details or right-click menu to save this mode and connect.
KeePass supplies the username in this mode; password and MFA responses are
entered in the terminal. Embedded SSH remains available in the connection editor.

SSH host keys are stored in the encrypted workspace by host and port. GateTree-Ops
marks first-seen keys as unknown and blocks silent replacement when a trusted
key changes. Active SSH transports send a keepalive every 20 seconds and are
closed after three unanswered keepalives.

The Agent can also be inspected directly during development:

```bash
src-tauri/target/debug/gatetree-ops-agent --start
src-tauri/target/debug/gatetree-ops-agent --status
src-tauri/target/debug/gatetree-ops-agent --stop
```

At workspace startup and after each Agent restart, GateTree-Ops requests the KeePass
master password if a database is configured in Vaults. A wrong password reopens
the prompt with the error. API and deployment accounts for unfinished plans are
reloaded into Agent memory without starting a remote job. The master password
is held only in the unlocked GUI workspace memory, scoped to the Agent process
and KeePass database. Job starts and retries reuse that session without another
prompt. Locking or restarting the GUI clears it; an Agent restart requires a new
unlock. The Agent page shows its version and protocol alongside the GUI
requirements, flags older Agents, and offers **Stop**, **Kill Agent**, and **Start**.

Process Config `steps` defines the exact Queue order. Only listed actions create
Queue entries: ticket inputs and `details` entries never add actions implicitly.
Jira/CX `validate` checks the specified ticket. OLAM `validate` is one endpoint
check: it verifies API authentication and configured template IDs/names using
GET requests, without launching Ansible jobs or expanding hosts/roles.

OLAM `deploy` requires an explicit inner workflow in Process Config:

```json
{"name":"Deploy OLAM","driver":"olam","action":"deploy","workflow":["validate","approve","deploy"]}
```

This creates one Queue row per Plan step, role, and host. Each row launches the
validation template, waits for operator approval, and launches the deployment
template for that same target and payload. Only then does the next row start.
Both validation and deployment templates must be configured in `details.olam`.
Missing or unsupported workflows fail configuration validation; nothing is added
implicitly. Use `["approve","deploy"]` explicitly for deployment without validation.
OLAM `validate_deployment` remains available as a standalone validation batch;
omit that separate Process step when using the combined workflow above.

Choose **Approve deployment** to launch, **Deny** to stop the Plan, or **Not now**
to leave it awaiting approval. Changed review content requires a fresh review.
Jira `update` sends only the comment and log attachments explicitly configured
in the step's `report`; see [Jira report configuration](docs/jira-report-config.md).
CX `update` uses the same report contract with `details.cx.requester`, Basic authentication, and the CX comments / base64 attachment endpoints; see [CX report configuration](docs/cx-report-config.md).
The **Help → Deployments → Config** menu lists report template variables, scopes, attachment sources,
and copyable JSON steps. The Process Config toolbar **?** button and **F1** in
the JSON editor open the same reference without replacing the editor draft.

Each Queue row has **Disable Job** / **Enable Job** controls. Disabled rows
remain visible and are skipped without running validation or deployment. Queued
jobs can be disabled while another row runs; stop an active job before disabling
it. A skipped position cannot be enabled during a run after later jobs started.
Disabling survives workspace and Agent restarts. Restart Plan preserves disables
for unchanged rows (matching name, action, target, and payload); new or changed
rows start enabled. Retry skips disabled predecessors and subsequent rows too.

A Queue job's **Restart** retries that configured action, preserves completed
predecessors, and queues subsequent steps again. **Restart Plan** rebuilds every
job from the latest Process Config and starts from the beginning. A new Plan
captures its Process Config; later edits do not change that Plan's run until a
full Restart. Legacy queues are preserved during reads and require a full
Restart before retrying individual jobs with the new execution model.

During validation and deployment, the Agent fetches the current job output every
10 seconds. Open job and Plan log panels refresh automatically, keeping your
reading position or following new output when scrolled to the bottom.

Both installer builds package the platform-native Agent executable beside the
GateTree-Ops GUI. The macOS build produces a universal Agent binary; the Windows MSI
contains the x64 Agent binary.
The build scripts and installer workflow also open the finished DMG/MSI and
fail unless the packaged `gatetree-ops-agent` executable is present.

The initial frontend has no package-manager dependency. It uses native ES
modules and Tauri's global API, so a UI framework can be introduced later
without changing the Rust workspace contract.

## Installers and updates

**Backup → Export encrypted backup** creates a `.gtbak` file protected by the
current workspace password, including when the workspace was unlocked using
biometrics. Export uses the unlocked key without asking for or saving that password.
Import asks for the source workspace password used at export time, which can differ
from the destination workspace password. Wrong passwords and altered files are
rejected before changing workspace content. Backups use AES-256-GCM with a fresh
HKDF-derived key and nonce per file. Existing unencrypted JSON backups remain
importable; leave the backup password blank for those files. Previously exported
JSON files remain unencrypted.

Installed releases provide **Help → Check for Updates**, version/release-note review,
signed downloads and coordinated GUI/Agent replacement. Install under
`~/Applications` on macOS or use the Windows per-user Setup EXE to update without
administrator privileges. Active deployments and SSH sessions block installation;
encrypted backups protect workspace updates and schema migrations. Debug builds
never replace themselves.

The **Build release** GitHub Actions workflow accepts a version and release notes,
builds macOS universal DMG/updater and Windows x64 Setup EXE/MSI on native runners,
and creates a **draft** in the separate public `GateTree-Ops-updates` repository.
Publish the reviewed draft to offer it to coworkers. Native build scripts remain
available for manual builds.

See [release setup and operating instructions](docs/releases.md) for signing keys,
GitHub secrets, first installation, local builds and the publication workflow.

## Workspace files

New workspace directories contain `gatetree-ops.sqlite` (SQLCipher-encrypted data)
and `gatetree-ops.sqlite.salt` (the master-key derivation salt). Existing
`pharos.sqlite` / `pharos.sqlite.salt` pairs are recognized in place, without
renaming or mixing database/salt pairs. Legacy settings and Agent plan history
migrate separately from the original GateTree application's data.

The master password is never written to disk. Release builds do not cache the
derived database key. Debug builds save a local `.gatetree-ops-dev-unlock` file
with user-only permissions, or read the existing legacy `.pharos-dev-unlock`,
so the development workspace can reopen automatically.
