# Local container Ansible

Add the Process in [Devcorp - Test example](examples/devcorp-test-container.json) under
Deployments → Ansible → Config. Its image reference is editable configuration.
Choose the local `orasys-mcp` project folder containing `inventory` and
`playbooks/deploy_orasys_role.yml`. For the older `orasys` checkout, change the
playbook to `playbooks/run_single_role.yml` and role_variable to `playbook_name`.

Create a Plan in the existing Planner: select this Process, the SSH Vault account,
one role, sites, targets, job grouping, steps and text extra vars. Jira/CX are
optional; their normal validation/report steps can be added to the Process.
Planner serializes local inputs under `inputs.ansible`; legacy `inputs.olam` and
`inputs.container` are also accepted. A Process with an OLAM driver still uses
its existing OLAM input and execution behavior.

The Process controls `engine` (`auto`, `docker`, `podman`), `image`, deployment
playbook, optional validation playbook, role variable, Vault password file,
optional `ansible_config`, and `timeout_seconds` (default 7200, max 86400).
The machine-local Ansible project folder is resolved at Plan start and saved in
the Agent's execution snapshot, not in shared Process config. The code/inventory
bind mount is read-only. Both existing inventory directory layouts are supported.
Do not edit the project while a validation/deployment is in progress.

Without `validation_playbook`, validation runs a generated SSH ping and hostname/IP
preview against the intersection of the approved limits, role and environment.
This checks connectivity; it does not simulate the deployment role. It uses the
selected inventories; repository `playbooks/group_vars` and `host_vars` are
linked beside the generated playbook inside the container for normal Ansible
variable discovery. A configured validation
playbook can perform deeper checks. A zero-host validation stops the Plan.

`workflow: ["validate", "approve", "deploy"]` keeps validation/approval/deployment
in one Queue row. `workflow: ["approve", "deploy"]` explicitly skips validation.
Deployment requires operator approval. The Agent records local job IDs and streams
container logs to Queue; Jira/CX reports include completed container jobs too.
Local jobs have no OLAM job URL.

The image must contain Ansible, `/bin/sh`, and its own Vault password file
(default `~/.orasys_vault_pass`, resolved inside the image). No Vault password is
read from the host or requested from KeePass for this runner. The Planner account
provides SSH/become credentials via short-lived private files, never command-line
passwords. The existing image `ANSIBLE_CONFIG` is retained when `ansible_config`
is omitted. Ansible can also discover `/workspace/ansible.cfg` normally.

The local engine needs network access to registry and SSH targets. Authenticate
the registry in Docker/Podman beforehand if necessary. Stopping a Plan cancels
engine commands and removes its named container. On Agent restart, interrupted
local containers are removed and their Plans remain stopped for explicit restart.
GUI closure alone does not stop the independent Agent.

Validation: fake-engine integration tests cover live output, approval, failures,
secret handling and cancellation without contacting deployment targets. An
explicit local-image smoke test also runs both phases against a temporary
localhost-only inventory, using a debug-only synthetic deployment playbook.
This checks the real image, embedded Vault file, secret copying and variable
discovery without deploying to real inventory targets.
