GateTree-Ops

- Unreleased: Add a validated 18-query Kafka rolling-upgrade preset for three brokers (IDs 1,2,3; kafka1/kafka2/kafka3 DNS names), including source freshness, scrape health, current-node JDK 25.0.5.1, ISR, leadership, recovery and failures. Prepared workspace Query Sets activate on the next schema upgrade without changing existing Plans.

- Unreleased: Config → Thanos Queries now supports reusable Query Sets. Planner can gate each host on all node- and cluster-scoped checks passing in consecutive complete rounds, with shared evaluation timestamps, minimum result counts, result age checks and a common timeout. Query Set policy and membership are frozen in each Plan; saved query text stays live. Missing queries/results, partial responses and failures block progress. Queue shows round progress and named check results in its logs. Existing single-query Plans retain their behavior.

- Queue retries transient OLAM status requests and offers a Recheck OLAM status icon after monitoring failures. Rechecking keeps the existing remote job and approval, without redeploying; confirmed OLAM success remains successful if output retrieval fails.

- SSH connections have a saved local Tunnels editor with custom source ports and destination hosts/ports. Rules use loopback-only OpenSSH forwarding and close with the SSH session; saving rules selects Terminal login. Reconnect to apply changes. Busy source ports stop login with a terminal error.

- Planner selects a saved Thanos query whose name matches the selected role when adding a Thanos Query step. Manual selections are preserved, and unmatched names stay unselected.
- Tasks require only a Jira key and roles. The optional name defaults to Jira; version fields and overrides are removed, with deployment versions resolved by Ansible for approval.
- Task links no longer require or compare explicit IMAGE_VERSION variables. Task-created Plans preserve existing extra variables and use Ansible's configured image versions.
- Task Plan controls fit on one row: Create Plan icon, existing Queue Plan selector and Save link icon, with accessible labels and tooltips.
- Task Queue Plan selections survive refreshes and failed link attempts. An explicit Save link button saves the association, with validation errors beside the selected Plan.
- Search fields disable automatic spelling correction, capitalization and autocomplete so technical names remain as typed.
- Deployments → Ansible → Tasks tracks planned roles by Jira, with separate deployment and Thanos validation progress. Create or link Queue Plans from a Task, or select an optional Task/service in Planner Builder; execution stays in Queue.
- Tasks uses compact typography consistent with Queue for headings, buttons, empty states and forms.
- Tasks are shown as expandable table rows with deployment counts. Service roles come from the local Ansible project.
- Image Hub Inventory can filter latest images by SLAPS status: All, Green (zero findings), or Red (findings); the filter combines with search, folder, project and repository type.
- Image Hub marks Ansible versions that differ from the latest image in yellow across all environments; matching versions stay green.
- Deployment rows now have an SSH terminal icon beside the target host. It asks for a username in an empty field and opens a separate GT terminal tab while the deployment continues.
- Deployment SSH uses interactive password/MFA login without reusing deployment or KeePass credentials, offering SSH keys, or creating saved Connections.

The GUI and Agent update together. Finish or stop active deployments and close SSH sessions before installing an update.
