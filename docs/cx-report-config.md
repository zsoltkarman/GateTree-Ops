# CX update configuration

A CX update uses the same explicit `report` contract as Jira. Add the step after
its deployments in Process Config `steps`. With Plan scope, one deployment or many
produce one summary comment and one combined ZIP log attachment.

```json
{
  "name": "Update CX",
  "driver": "cx",
  "action": "update",
  "requester": "operator@example.test",
  "report": {
    "scope": "plan",
    "comment_template": "Plan #{{plan_number}}: {{plan_name}}\n\n{{deployments}}",
    "deployment_template": "{{role}} · {{step}} · {{target}} / {{ip}} · {{status}} · Job #{{job_id}} · {{datetime}}",
    "attachments": [
      {
        "source": "plan_log",
        "compression": "zip",
        "filename_template": "{{request}}-plan-{{plan_number}}.zip"
      }
    ]
  }
}
```

Keep the CX connection in `details.cx`:

```json
{
  "url": "https://cx.example.test/requests",
  "account": "account_cx",
  "verify_tls_certificate": true
}
```

These are illustrative values; use your existing endpoint, account reference and
requester. Configure the default requester on the Update CX step, alongside
`action` and `report`. A Plan value can override it:

```yaml
inputs:
  cx:
    request: "REQ-100"
    requester: "operator@example.test"
```

Requester priority is **Plan `inputs.cx.requester` → update step `requester` →
legacy `details.cx.requester`**. Blank values fall through to the next default.
Each Update CX step has its own requester; it belongs outside `report`. An update
requires a nonempty effective requester. Its resolved value is frozen per step
into the Process snapshot submitted to Agent; editing the Process later
does not change a running Plan. The account's KeePass username and password supply HTTP Basic
authentication. The request number comes from the Plan's `inputs.cx.request`.
For compatibility with the existing CX validation driver, certificate verification
is disabled when `verify_tls_certificate` is omitted; set it explicitly to `true`
when the endpoint's certificate is trusted.

## Templates and logs

All [Jira report fields and attachment options](jira-report-config.md) are shared.
`{{request}}` is the CX request number. `{{issue}}` also represents the current
target ticket, so an existing Jira report template can be reused with CX.
`{{request}}` is empty for a Jira update. Deployment fields belong in
`deployment_template` for Plan scope; `{{deployments}}` inserts the rendered list.
Unknown placeholders and unsupported compression fail before any update is sent.

`plan_log` combines the preceding enabled Queue rows into one UTF-8 `.log` inside
the ZIP. `scope: "deployment"` is also supported, with `deployment_log` or
`job_log`, but creates one report per completed deployment. Use Plan scope to
keep the ticket concise. Omit `attachments` for a comment only, or omit
`comment_template` for attachments only.

The update includes only preceding completed OLAM deployments with a deployment
job ID; disabled rows are excluded. A failed deployment stops the linear Plan
before a later update step. No report is added implicitly. The update does not
change the CX status, close the request, or notify CX at each validation/approval.

## CX transport and retries

The existing GateTree CX integration's API contract is preserved:

- First `POST {url}/{request}/attachments?requester=…` with a JSON array containing
  `fileName`, base64 `data`, `contentType`, and `requester`. ZIP uses `application/zip`.
- Then `POST {url}/{request}/comments` with `comment` and `requester` in JSON.

Successful response IDs, or final HTTP acknowledgements when CX returns no ID,
are persisted per operation. Empty success responses and JSON objects/arrays are
supported. HTTP 202 is treated as an unconfirmed asynchronous operation.
Retrying only the update job skips already confirmed writes. A definite client
rejection (except HTTP 408) can be retried after correcting the cause. Connection
failures, server errors, incomplete/unreadable responses and interrupted writes
remain unconfirmed: inspect the request before taking further action; the Agent
will not blindly resend them.

A full Restart Plan starts a new run, reruns its deployments and clears report
progress. It may send another report. Process changes affect an existing Plan
only after a full Restart; add the step before creating the next Plan.

**Help → Deployments → Config** includes copyable CX examples and the complete
variable reference.
