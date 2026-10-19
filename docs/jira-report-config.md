# Jira update configuration

Put the update step at the desired position in Process Config `steps`. A report
runs only when execution reaches that step. A failed deployment stops the linear
Plan before a later report. Nothing posts simply because Jira details exist.

Keep Jira connection details in `details.jira.url` and `details.jira.account`.
The account uses the existing KeePass Personal Access Token authentication.
The issue comes from the Plan's `inputs.jira.issue`.

## Comment and log for each completed deployment

```json
{
  "name": "Update Jira",
  "driver": "jira",
  "action": "update",
  "report": {
    "scope": "deployment",
    "comment_template": "Following role has been deployed:\nRole name: {{role}}\nTarget: {{target}} / {{ip}}\nDatetime: {{datetime}}\nStatus: {{status}}\nJOB ID: {{job_id}}\n{{job_url}}",
    "attachments": [
      {
        "source": "deployment_log",
        "filename_template": "{{issue}}-{{target}}-{{job_id}}.log"
      }
    ]
  }
}
```

The single Jira Queue row reports each preceding completed OLAM deployment with
an external deployment job ID. Disabled rows and standalone validation rows are
excluded. It uploads the attachment(s) before posting the corresponding comment.

Omit `attachments` for comments only. Omit `comment_template` for attachments
only. At least one must be configured. There is no built-in comment text or
automatic attachment.

## One Plan summary and attachment

```json
{
  "name": "Update Jira",
  "driver": "jira",
  "action": "update",
  "report": {
    "scope": "plan",
    "comment_template": "Plan #{{plan_number}}: {{plan_name}}\n\n{{deployments}}",
    "deployment_template": "Role: {{role}} | Target: {{target}} / {{ip}} | Datetime: {{datetime}} | Status: {{status}} | Job: {{job_id}}",
    "attachments": [
      {
        "source": "plan_log",
        "compression": "zip",
        "filename_template": "{{issue}}-plan-{{plan_number}}.zip"
      }
    ]
  }
}
```

`{{deployments}}` joins the configured `deployment_template` once per completed
preceding deployment. This placeholder requires `scope: "plan"` and an explicit
`deployment_template`.

The ZIP contains one combined UTF-8 log file with a separate section for each
preceding enabled job. This works for both a single deployment and multiple
deployments: the ticket receives one comment and one ZIP attachment. Compression
is explicitly configured and applies regardless of log size. Omit `compression`
or use `"compression": "none"` for a plain `.log` attachment. `"compression": "zip"`
requires a filename template ending in `.zip`.

## Template fields

| Field | Value |
| --- | --- |
| `{{issue}}` | Jira issue key (CX request number when used in a CX report) |
| `{{request}}` | CX request number; empty for Jira |
| `{{plan_name}}`, `{{plan_number}}` | Plan identity |
| `{{role}}` | Deployment payload `playbook_name` |
| `{{target}}` | Selected target FQDN/hostname |
| `{{ip}}` | `Ansible ssh host` from validation output, empty if unavailable |
| `{{datetime}}` | Recorded deployment completion time in UTC; `unknown` for older jobs without a timestamp |
| `{{status}}` | Deployment status |
| `{{job_id}}` | OLAM deployment job ID |
| `{{validation_job_id}}` | OLAM validation job ID, if present |
| `{{step}}` | Deployment Queue row name |
| `{{job_url}}` | OLAM deployment output link |
| `{{deployments}}` | Configured summary list, Plan scope only |

Deployment fields belong in a deployment-scoped comment or `deployment_template`.
For the Plan-scoped comment, use Plan fields, `{{deployments}}`, and `{{datetime}}`
(the report generation time). Unknown placeholders fail validation before posting.

Attachment sources:

- `deployment_log`: selected deployment output, excluding the preceding validation
  section when the Agent has its deployment boundary.
- `job_log`: the selected deployment row's entire log, including validation.
- `plan_log`: logs of all preceding enabled rows, available for Plan scope only.

Names are rendered from `filename_template`, stripped of path separators and
unsupported filename characters, and limited to 180 characters. Logs are UTF-8 text; ZIP attachments use `application/zip`. There is no arbitrary filesystem upload.

## Retry and configuration changes

Confirmed Jira attachment and comment IDs are persisted with the report job.
Retrying that job reuses them and sends only remaining operations. A rejected
client request can be retried after correcting its cause. If the connection fails
with an uncertain outcome, or the Agent stops during a write, the job requires
operator inspection and will not automatically repeat that uncertain operation.
This avoids blindly duplicating comments or attachments.

A full Restart Plan is a new run and clears report progress along with other
job state. It can send a new report after running the deployment steps again.
Process edits affect existing Plans only after a full Restart, as for other steps.

The transport follows the Jira Data Center REST API for
[issue comments and attachments](https://developer.atlassian.com/server/jira/platform/rest/v10000/api-group-issue/).

CX uses the same report contract with its own connection and transport; see [CX report configuration](cx-report-config.md).
