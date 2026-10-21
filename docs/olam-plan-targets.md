# OLAM Plan targets

Plans use `plan_name`, `process` and nested `inputs`.

The existing `inputs.olam.targets.hosts` list is unchanged: the Queue creates
one job per host, role and Plan step, with a single-item `ansible_limit`.
The strings can also be inventory selectors; they still run separately in this mode.

Use `inventory_groups` instead of `hosts` when all groups should be submitted
together in one OLAM job:

```yaml
plan_name: Service deployment
process: Deployment process
inputs:
  jira:
    issue: APP-123
  cx:
    request: "REQUEST-ID"
  olam:
    strategy: linear
    account: deployment_account
    targets:
      roles:
        - service_role
      inventory_groups:
        - group_one
        - group_two
    steps:
      - name: Upgrade service
```

For one role and one Plan step this creates one OLAM Queue row with
`playbook_name: service_role` and `ansible_limit: [group_one, group_two]`.
Multiple roles or Plan steps still create separate rows. Credentials and
environment continue to come from the existing account and Process settings.

The Process must explicitly configure its OLAM deployment workflow, for example
`workflow: [validate, approve, deploy]`. The full group list is kept through
validation, approval and deployment; the validation preview lists the hosts
reported by Ansible. GateTree-Ops does not independently expand or snapshot group
membership. Inventory changes between validation and deployment can therefore
change the resolved machines, just as in a direct OLAM group launch.

`hosts` and `inventory_groups` are mutually exclusive and cannot both be empty.
Group entries must be nonempty. No group names are inferred from `sites`;
provide the exact inventory group names.

## Local Planner inventory preview

Choose **Local Ansible folder → Choose folder** in Deployments / Config. Planner Builder reports a missing folder and reads the saved setting. Select the Ansible project containing `inventory/`. The machine
preference lives in `local-ansible.json` inside Tauri's application config
directory. It survives restart independently of the workspace. It is not
included in workspace backups and does not change the Process JSON.

The Planner selects the environment from
`details.olam.payload_defaults.ENVIRONMENT`. It reads static site inventories
under `inventory/gbucs_orasys_host_inventory/<environment>/<site>/`, or under
`inventory/<environment>/<site>/` if the intermediate directory is absent.
Role memberships come from `roles.yaml` / `roles.yml`; host addresses from
`ansible_host` or `ansible_ssh_host`; both `location.yaml` and `locations.yaml`
are supported. Group children are resolved, and matching hosts are intersected
with the environment and actual `<environment>_<site>` group. If the site group
is absent, the preview reports it and only produces explicit host targets.

The Builder allows one role and multiple sites. Role/site selections and host exclusions stay in the in-memory Builder draft until **Save Plan**.
**Jobs** defaults to **Separate job per host**: all selected hosts are saved
explicitly, producing one job per host and deployment step. **All hosts in one
job** passes full site groups together, or a combined explicit host list when
hosts have been excluded. The choice is stored as `targets.job_mode`
(`per_host` or `together`). Refresh shows a notice when the inventory content changes.
The local parser does not execute Ansible, inventory plugins, scripts or deployments.
OLAM still resolves its own inventory during validation and execution; this
preview does not freeze group membership or replace approval-time validation.

## Steps and saving

Add one or more ordered deployment steps in Planner Builder. Each step has a
name and an optional extra variables textarea. Enter one `NAME: value` or
`NAME=value` assignment per line; multiple lines can be pasted together. All variable values are stored as text, including numbers, booleans and JSON-like
content. Duplicate variable names and missing names block saving. Move steps up/down to change their
execution order. Execution strategy is selectable as Linear or Free.

**Save Plan** builds nested Plan YAML with `inputs.olam.steps[].extra_vars`,
looks up the selected Process and Vault account by their references, and saves
through the existing Queue service. Jira and CX ticket fields are optional. A missing ticket omits that driver’s
validation and update jobs from the Plan. CX numbers preserve leading zeroes.
The created Plan opens in Queued state. Saving does not start jobs. A failed
save retains the form and variable rows for correction or retry.

## Explicit job grouping

```yaml
targets:
  job_mode: per_host
  roles: [service_role]
  hosts: [host_one, host_two, host_three]
```

With two Plan steps this generates six deployment jobs, in step-then-host order.
Use `job_mode: together` with the same hosts for two jobs instead, each carrying
all three hosts in `ansible_limit`. Together also supports `inventory_groups`.
Per-host mode requires `hosts`; group names are not expanded by Queue generation.
Without `job_mode`, existing Plans keep their behavior: `hosts` are separate,
`inventory_groups` are combined. Changing the Builder selection applies on Save;
already-saved Queue plans retain their generated jobs until explicitly edited
or recreated.
