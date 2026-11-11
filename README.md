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
Terminal login allows only keyboard-interactive and password authentication; it
does not offer SSH keys, certificates or agent identities. It permits one password
prompt per authentication method and one connection attempt. Embedded
keyboard-interactive login sends its stored password to at most one prompt;
additional prompts require interactive Terminal login.

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
check: it signs in and verifies configured template IDs/names without launching
Ansible jobs or expanding hosts/roles.

All OLAM environments use the same authentication flow as the Ansible MCP:
the Agent reads the configured KeePass username/password, signs in through
`/api/login/` with session cookies and CSRF protection, and requests an API token
from `/api/v2/tokens/`. Endpoint checks, template checks, job launches, status and
stdout requests then use Bearer authentication. The token stays in the API
client's memory for that operation; it is not saved in KeePass or the workspace.
Existing Process Config and Plan account references continue to work.

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

Pre-update backups are saved under the application's local data folder in
`workspace-backups`, including the encrypted database, matching salt and Agent
state. Update backups use the salt retained when the workspace was unlocked,
so a cloud provider evicting the salt file cannot block the salt-copy step.
Migration backups remain beside the workspace.

The **Build release** GitHub Actions workflow accepts a version and release notes,
builds macOS universal DMG/updater and Windows x64 Setup EXE/MSI on native runners,
and creates a **draft** in the separate public `GateTree-Ops-updates` repository.
Publish the reviewed draft to offer it to coworkers. Native build scripts remain
available for manual builds.

See [release setup and operating instructions](docs/releases.md) for signing keys,
GitHub secrets, first installation, local builds and the publication workflow.

## Workspace files

**Image Hub** stores a repository inventory in the encrypted workspace. Right-click
the inventory or a folder to add folders, subfolders or repositories. Add and edit
projects under **Image Hub → Config → Projects**. Repositories have a name, local path, OCID,
repository URL and type (CloudLab, SCM, GitLab or GitHub). Only the name and type are
required. Right-click a repository to edit it or move it to another folder; use
**Edit folder** to rename or move a folder. Non-empty folders cannot be deleted.
Removing a repository from the inventory keeps its local files. Inventory data
is included in workspace backups. This initial module manages repository
metadata, published image versions and local security report snapshots.

**Image Hub → Config** owns all environment-specific endpoints, URL templates,
artifact repository, team, region, context and SCM profile/authentication mode.
New workspaces have no built-in service addresses. Enter the values directly or
use **Import JSON…** to load a configuration file. Settings are encrypted with
the workspace and included in backups. Changing configuration clears cached
artifact lookups so values from another environment cannot be reused.

Inventory combines all repositories across every folder and project in one table,
including their folder/project, latest image, AF, SLAPS and Build Logs links.
Search by repository, folder, project or local path, and combine folder (including
subfolders), project and type filters. Sort by folder/repository or repository
name. Project source settings and add/edit actions live in Config; selecting a
project in Inventory filters the list and retains its Sync Project action. The full
inventory also offers **Sync all projects**: one native background batch processes
all configured projects in order, with an overall percentage and per-project
SCM/SLAPS/AF results below the list. A project failure does not skip the remaining
projects. Navigation can change while the batch continues; concurrent syncs are
blocked until it finishes.
Each image initially shows only its Latest row. The arrow expands Ansible rows
for every environment found in the local roles. Set `ansible_dir` in Config to
an absolute Ansible project directory. Image names and versions are read from
role defaults, `<role>_global.yml` and `<role>_<environment>.yml` (also `.yaml`),
in that precedence order. Missing environment overrides inherit the global
value. Literal variable aliases are supported; other Jinja expressions display
an unavailable version. No playbooks run. These are deployment configuration
values, not observations of running containers. Each environment has its own AF
version link and report icons. Ansible AF versions are green when they match the
latest collected image and red when they differ; unavailable latest versions
remain neutral. Base report URLs open repository overviews;
version-specific report templates can use `{version}`. No build or scan ID is
inferred from a version. Reopen Image Hub or Sync Project to reread Ansible files.

Sync Project queries the configured artifact folder API in the background.
The list displays the latest stored published version while sync runs. It chooses the greatest numeric three-part release tag
that contains a Docker manifest, and links that version using the configured
artifact page template. Lookups are cached for one minute. A missing publication
is distinct from a failed request. On a failed refresh, the last known version
remains visible with a warning and the failure reason.

SCM Config keys `scm_profile` and `scm_auth` select the OCI CLI identity and
mode (the existing `scmProfile` and `scmAuth` names also work). They contain
profile names and authentication modes, not credentials.

Sync Project reports expired OCI authentication separately from repository access
failures. Optional Config entry `security_auth_command` supplies the terminal
command shown in that warning; GT never executes that command automatically.

Sync Project also downloads Anchore reports for unique image/version pairs
from Latest and matching Ansible roles. Config `slaps_api_url` is the HTTPS
SLAPS service base URL, and `slaps_realm` selects the realm scope for stored
report snapshots. Requests use `scm_profile` / `scm_auth`. No service addresses
or profile names are built into the source. The sync result reports how many
reports were saved and any authentication or scan errors. The raw JSON,
top-level report sections and indexed finding fields are stored in encrypted
workspace SQL with history. A failed download is distinct from an empty report.

Click the SLAPS shield to view the local findings table: Severity, Issue,
Package, Installed, Fixed and Gating. Rows are paged to fit the available height;
long values are truncated with full text in tooltips. The report view retains
an Open SLAPS browser link. Downloading an Anchore report does not establish a
realm-specific release approval. The shield is green when the downloaded report
has no findings and red when it has findings. Missing, pending or failed reports
keep a neutral shield; the approval status remains separate.

Projects have a name, an absolute local directory and a type (SCM, GitLab or
CloudLab). Use **Move to folder…** from a project’s context menu or its Config
action to change its Image Hub folder together with its repositories, without
starting a sync or moving local files. The optional **Project URL / ID** identifies the remote source: an
OCI DevOps project OCID or Console project URL for SCM, or a GitLab-compatible
group URL for GitLab/CloudLab. Supplying a full SCM project page URL also
resolves each repository's browser link automatically from that URL's origin
and context plus the repository OCID collected during sync. No repository URL
template is needed in that case, and clone URLs remain unchanged. An OCID-only
project source needs a configured browser URL template to identify the console.
Without this field, sync imports local Git
repositories only. **Save and sync project** discovers repositories immediately;
**Sync Project** starts a background job and immediately returns control to the
interface. Its button displays progress as a percentage. Under the list, SCM,
SLAPS and AF each show their own completed/total counters and details
for failures or missing configuration. SCM Details lists every failed repository
by name with its error, including while sync is running. Switching views does not stop the job.
Locking or changing workspace stops writes from the old session; restarting GT
marks interrupted jobs so the next sync can continue saved work.

Build Logs are browser links configured by `build_log_url`. Sync Project does
not fetch build history or download logs. SLAPS reports are collected for the
latest AF image and the configured Ansible versions. Historical reports are
reused for one hour; latest-image reports are refreshed on sync.

Project sync calls the installed `oci`, `glab` and `git` executables without
Codex or npm dependencies. Choose the SCM profile and authentication mode in
**Image Hub → Config**, configure the appropriate host in `glab`, and set up
Git/SSH authentication in your terminal first. Git SSH connections respect the
host-key policy and identity settings in your existing SSH configuration and
run without interactive prompts. Sync never launches a login flow, refreshes
credentials, or stores credentials in project URLs. GitLab-compatible groups include subgroups and all result pages.
OCI Console URLs can supply an explicit `region` query parameter; a bare OCID
uses the configured region or the selected CLI profile's region. Git uses the clone URL returned by the
provider. Missing clones are created below the project directory, preserving
subgroup paths. Existing repositories are checked against their origin and
kept as they are; sync does not pull, reset, switch branches or delete files.

New workspace directories contain `gatetree-ops.sqlite` (SQLCipher-encrypted data)
and `gatetree-ops.sqlite.salt` (the master-key derivation salt). Existing
`pharos.sqlite` / `pharos.sqlite.salt` pairs are recognized in place, without
renaming or mixing database/salt pairs. Legacy settings and Agent plan history
migrate separately from the original GateTree application's data.

The master password is never written to disk. Release builds do not cache the
derived database key. Debug builds save a local `.gatetree-ops-dev-unlock` file
with user-only permissions, or read the existing legacy `.pharos-dev-unlock`,
so the development workspace can reopen automatically.

Queue polling runs database and Agent reads off the UI thread. Active visible
queues refresh every 1.5 seconds; idle queues refresh every 5 seconds. Hidden
panes and background windows pause polling. Routine snapshots omit log contents;
opening a log loads it separately, with subsequent reads returning only the new
text when the previous prefix still matches. Replaced or restarted logs reset
safely. SQL rows are updated only when their values change. Status refreshes
preserve existing controls, open log nodes, column widths and unsaved payload
edits. Full log snapshots remain available for approval and report recovery.

Within each Plan, jobs are grouped by status with counts. Running (including
awaiting approval) and Failed / Stopped are expanded initially; Queued, Completed
and Skipped are collapsed. Each group's expansion is retained during refreshes.
Grouping preserves execution sequence numbers and original job dependencies. A failed Thanos check also brings its
completed deployment into Failed / Stopped, with **Restart deploy + Thanos**.
That action launches a fresh Ansible execution and reruns only its following
same-target Thanos checks. Approval gates remain in place. On an inactive Plan,
the retry scope is persisted across Agent restarts and other jobs or Jira/CX
updates are not restarted. Healthy lanes already running in parallel continue.

Use **Archive Plan** on a successfully completed Plan to move it from Success
to **Queue → Archived**, the last Queue group. Archived Plans are omitted from
the normal Queue, retain their original jobs/payloads/logs, and are read-only.
They can still be opened or copied into a new queued Plan. The archive marker is
stored in the encrypted workspace metadata and survives Agent synchronization,
workspace reopening and backups.

### Copy a completed Plan

Use **Copy to new Plan** in a completed Plan's header. The Planner opens with the
source Jira/CX numbers, requester, account reference, role, strategy, region concurrency, deployment
steps, extra variables and Thanos query/version. Select a Process and fresh
sites/hosts before saving. An old step-level `ENVIRONMENT` override is omitted so
the selected Process supplies the destination environment. Review the account
and other extra variables for that environment. Unchanged copied extra variables
retain their original types; editing their text uses the normal text editor rules.

Save creates a new queued Plan with a new number and current Process Config.
The original Plan and its logs are preserved. Host selections, job IDs, job
results and approvals are not inherited. Start the new Plan separately.

### Multi Role Plan

The **Multi Role Plan** tab is temporarily hidden. The Planner opens the single
role builder, including when a saved draft previously selected Multi Role.

Multi Role support shares a group name, Process, Vault account, Jira/CX references, execution strategy
and parallel-region limit. Use **Add Role** for each service. Every role has its
own inventory sites/hosts, optional extra variables and optional saved Thanos
query; queries are previewed read-only and maintained under Config.

**Create Plans** validates every role before committing a single SQL transaction.
A failure leaves the Queue unchanged. Retrying the same successful request returns
the original Plan IDs. Each role becomes a separate queued Plan under a shared
Queue group and must be started separately. Existing per-host Deploy → Thanos
ordering, approvals, regional isolation and report steps apply to every Plan.
Parallel regions is a limit per role Plan. Tab switching preserves both drafts.

### Saved validation queries

Under **Deployments → Config → Thanos Queries**, use **Add Query** to store a
name and PromQL template. Queries are independent of deployment roles. Edit or
delete entries in the same list. Queries live in the encrypted workspace SQL database and are included in
workspace backups and the deployment config bundle. Editing rejects a stale
revision instead of silently overwriting a newer change.

Thanos steps are optional; a Plan may contain only Deploy steps.
In a Planner **Thanos Query** step, select a saved query by its name.
The selected PromQL appears in a read-only preview.
The Plan stores the saved query ID and a preview of its original text. Every
Thanos HTTP attempt uses the latest saved query from **Config → Thanos Queries**,
including retries and polls of a running check. Saving a query refreshes queued,
failed and running checks in the Agent; starting or retrying also refreshes them.
Host, region and expected-version values remain bound to the Plan's deployment.
`${DEPLOY_IMAGE_VERSION}` still comes from the completed deployment's log.
Completed jobs keep their results and logs; changing a query does not rerun Deploy.
Deleting a referenced query or removing a required placeholder fails validation
rather than using the old text. Legacy Plans are linked by a unique exact query
match or saved-query name; if neither identifies the source, edit the Plan and
select its saved query again. Copied Plans retain the saved query ID.
The Planner only offers saved queries; selecting one does not overwrite an
existing step name or expected version. Polling and connection settings stay in
Process Config under `details.thanos`. After updating the application, restart
the Agent to use the updated IPC protocol.

### Thanos Query in Planner Builder

Each Planner step can be **Deploy** (existing extra variables) or **Thanos Query**
(name, PromQL query, and an optional expected exporter version). Add connection and polling settings to
the selected Process under Deployments → Config:

```json
"thanos": {
  "url": "https://thanos.example.test",
  "auth": "none",
  "timeout_seconds": 300,
  "interval_seconds": 30
}
```

This object belongs inside the existing `details` object. `url` is the Thanos
base URL; the Agent calls `/api/v1/query` directly. `auth` accepts `none`, `basic`
or `bearer`; the latter two require an `account` reference from Vaults. No endpoint
or organization-specific settings are built in.

A query succeeds only with nonempty instant results whose values all equal `1`.
Zero, other values, empty results or partial-data warnings retry until timeout.
For example `vector(1)` verifies connectivity, not deployment correctness. Write
the query to check the required version, target coverage and freshness. An
application build-info version is not necessarily the container image tag.
Inside double-quoted query strings, `${DEPLOY_REGION}` is replaced with the
deployment site's region, and `${EXPECTED_VERSION}` with the step's
**Expected exporter version** field. Region placeholders require a preceding
Deploy step and selected sites. `${DEPLOY_IMAGE_VERSION}` instead uses the
resolved role `IMAGE_VERSION` from the matching host's completed deployment
validation log; it requires **Separate job per host** and a preceding Deploy.
Missing, ambiguous or unresolved version evidence fails the check. Use it only
when the application's metric version matches its container image tag.
With **Separate job per host**, the following
Thanos Query steps run immediately after each host's deployment, before the next
host starts. Add `instance=~"${DEPLOY_HOST_REGEX}"` to every such query: this
placeholder matches only that deployed hostname, with an optional URI scheme
and port. Hostname dots and other regex characters are escaped. The Builder
saves `targets.host_sites` so each host gets only its own site's region; older
single-site Plans can use their single selected site. Missing multi-site context
is rejected rather than validating a different region.

Set **Parallel regions** in Planner Builder to run up to that many deployment
sites together (1–16, default 1). Values above 1 require **Separate job per host**
and a host-to-site mapping from the selected inventory. The Plan saves this as
`inputs.olam.parallel_regions` (or `inputs.ansible.parallel_regions` for a container
Process). Within each region, each Deploy and its following Thanos checks finish
before the next host starts. A region holds its slot while waiting for approval;
when it finishes, the next region starts. Process steps outside the regional jobs,
including Jira/CX validation and updates, remain sequential barriers.

If a deployment or query fails, only that region stops: its failed row is kept,
and its remaining jobs become Stopped. Healthy regions continue their Deploy →
Thanos sequences, including regions waiting for an available slot. Failed regions
release their slots. Approvals remain available in healthy regions. Once all
healthy regions finish, the Plan retains its failure status and does not send
final success updates. Global prerequisite failures still stop the whole Plan.
Stop stops monitoring all active lanes; remote OLAM jobs retain their IDs. Retrying a failed step preserves completed regional work, and stopped OLAM
monitors reconnect to their existing jobs. Completed regional jobs require a
copied Plan for a fresh run, except that a deployment paired with a failed Thanos
check can be retried individually. Use matching GUI and Agent builds for this feature.
OLAM status polling retries HTTP 401 up to three consecutive requests; a
successful status response resets the counter. These retries only read the
existing job and never repeat the launch POST. If polling still fails, GT
separately attempts to retrieve the Ansible output and includes the OLAM job link. A monitoring failure leaves the remote
result unknown; an available Ansible log is shown alongside the API error.

The host matcher is required for per-host queries to prevent a region-wide check
from passing because a different host already has the expected version. Standalone
queries before any deployment retain their literal behavior. **All hosts in one
job** retains batch validation after the combined deployment; host placeholders
require Separate job per host. Sites with all hosts excluded are not queried.

By default the region equals the site name. When metric labels differ, add a
site-to-region map to `details.thanos` in that environment's Process Config:

```json
"region_map": { "west": "west_staging" }
```

For example, with expected exporter version `1.97.0`:

```promql
(
  count(redis_exporter_build_info{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}",version="${EXPECTED_VERSION}"})
  /
  count(redis_exporter_build_info{region="${DEPLOY_REGION}",instance=~"${DEPLOY_HOST_REGEX}"})
) == bool 1 or vector(0)
```

This example requires data for the host and the expected version on all currently
visible matching series. It does not prove missing targets or scrape freshness.
The exporter version must be supplied explicitly; the image tag is not converted.
Unknown, missing or unquoted placeholders prevent the Plan from being saved.
Literal queries without placeholders retain their existing behavior. Process-level
query steps still require complete literal queries.

Invalid queries/authentication fail
immediately; Stop interrupts polling. Query responses and UTC attempt timestamps
are saved with the Queue job in the workspace.

Mixed Planner steps require exactly one deploy step in Process Config. Queries
immediately following a Deploy step run for each host in sequence, or after the
batch in together mode (once per region when using `${DEPLOY_REGION}`). Zero/no
data keeps that check polling; the next host remains queued. A timeout or fatal
query error fails the Plan and leaves later jobs queued. Existing Plans with no step type keep their Deploy
behavior. Existing Jira/CX reporting can attach `plan_log` to include query
results; no ticket update is added automatically.
