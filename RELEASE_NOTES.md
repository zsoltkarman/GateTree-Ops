GateTree-Ops

- Jira report Retry can recover unconfirmed operations after the operator checks the issue and explicitly confirms resending missing operations. Confirmed uploads/comments and completed deployments are preserved. Connection failures before sending are retryable directly.

- Agent health checks remain responsive during SSH connection setup. A timed-out or disconnected GUI request no longer stops the Agent.

- Per-Plan Approve all can be enabled or revoked while queued or running. Validation still runs; revoking restores manual approval for upcoming deployments without interrupting an approved deployment.

- Configurable deployment workflows with validation, operator approval, deployment and Jira/CX reports with compressed log attachments.
- Live deployment status and job logs, individual job disabling, and restarting stopped Plans from the beginning.
- Global search with a compact result table, full folder paths and a side preview.
- Connection quick launcher, independent SSH session tabs, local terminal themes and inherited KeePass account settings.
- Queue navigation shows each Plan's status. Completed deployments leave the Agent activity list while remaining available in the Queue.
- Signed automatic updates for installed releases, encrypted workspace backups and per-user Windows setup.
- Fixed startup with an empty Quick Launcher after workspace authentication. Missing update feeds and workspace import I/O errors now show clearer recovery guidance.
- Updates now accept writable system installation folders, including /Applications, and immediately show progress or an explicit failure.

The GUI and Agent update together. Finish or stop active deployments and close SSH sessions before installing an update.
