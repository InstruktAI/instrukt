---
description: Local-leading work records reflected to required GitHub issues with versioned native artifacts and revision checks.
visibility: public
delivered_by: [instrukt-first-build]
---
# GitHub Record — Spec

## What it is

A developer works through a local record while GitHub holds its shared,
recoverable reflection. Work accepted locally remains available when GitHub is
unreachable. A collaborator reads the durable artifacts and decisions without
needing the conversation that produced them.

## Canonical fields

### Local-leading pull-through

A *local item* is a todo present in this computer's local record. Reads and
listings of work not in this computer's local record read GitHub directly
and store nothing. An item enters the local record when dispatched here or
when a command changes it here. Reads and writes of local items use that local
record. A write commits there
and is reflected to GitHub when reachable. Failure to reflect does not undo a
locally accepted write. The record distinguishes pending reflection from a
confirmed remote revision; local success does not claim remote completion. While GitHub is unreachable, listings show local items only and say that
GitHub is unreachable.

GitHub is required, rather than a selectable storage backend. For local items, GitHub changes
are pulled with a conditional request on every runner tick and before any
command on that item, entering the local store before readers consume them. Rebuilding
from GitHub recovers reflected work, subject to the artifact history GitHub
still serves. Unreflected local writes have no remote recovery guarantee.

### Issues and visible work

Every todo, including a task, an iceboxed todo and a structural holder, is
reflected as an issue in its project's GitHub repository. The issue shows the
current intent, kind where applicable, the record's status (active, iceboxed,
held, delivered or withdrawn; never a workflow phase), hold or icebox state,
group relationships and prerequisites, and beside each open approval gate a
short prefix of the current evidence digest. Delivery closes it as completed and
names the delivery evidence; removal closes it as withdrawn; restoration
reopens it.

A project board is optional. When `board` names a GitHub project
(`<owner>/<number>`), instrukt adds each reflected todo's issue to that project
and sets a single-select field named `Record status` that instrukt owns (never
the built-in `Status`) to the record's status: active, iceboxed, held, delivered
or withdrawn. It creates the field when absent and adds missing options; it
never removes or renames options and never touches other fields. Changing a
board position does not establish a workflow fact. Board reflection is retried
on its own and never blocks the issue or the local record.

Repository implementation has a linked pull request during its build and
review path. It carries the reviewed commits and recorded verdicts. If the
branch is not available on GitHub, creation remains pending. Delivery closes
the pull request with the delivered commit. The integrator owns merging and
the integration gate; pull-request state alone never proves delivery.

### Artifacts and stamps

Artifacts and stamps are the published work evidence. Conversational turns,
approval questions and other back-and-forth remain in the configured runtime.
An evidence reference may identify the originating conversation without
copying it into GitHub.

Each artifact is a native issue artifact with saved versions. An artifact's
identity is its recorded provider address, not its display name. Each version
retains its content digest, workflow writer and creation time. A saved version
is not rewritten to replace history. Implementation plans and task plans use
the provider's plan capability; other artifacts use its document capability.

A committed remote revision identifies the artifact versions and stamps it
contains. A version uploaded without a committed referencing revision is not
accepted as committed evidence. A similarly named artifact or an unrelated
edit does not replace a recorded version. Restoration verifies the recorded
content; missing or unverifiable evidence is reported rather than fabricated.

The `instrukt todo artifacts` command family provides `list <slug>`,
`read <slug> <name>`, `write <slug> <name> <content>` and
`diff <slug> <name> <old-digest> <new-digest>` through the local record.
Writing appends a version; diff compares retained evidence. Stamp evidence is
owned by `project/instrukt/spec/feature/work/stamps`.

### Writer and convergence

**Repository.** Reflection writes to the GitHub repository derived from the
URL of the remote the default branch tracks (the default branch is the
remote's `HEAD`, then `init.defaultBranch`, then `main`). The project-layer key
`github.repository: owner/repo` overrides it; when the remote is not a GitHub
repository of `github.api_url`'s host and the key is absent, todo commands
refuse and name that key.

**Credentials.** The token is read, in order, from `GITHUB_TOKEN`, `GH_TOKEN`
and `gh auth token`, and is never stored. With none, reflection reports
"pending: no credentials" and the local record continues.

A per-computer writer lock serializes reflection; no separate writer process
runs. Every command that writes the record and every runner tick flushes
pending reflection under that lock. Writes check the item's known
remote revision before replacing it. A newer remote revision is a conflict,
not permission to overwrite it with a stale local snapshot. The local change
is not reflected and the item is marked conflicted. Commands on that item
refuse with the door `instrukt todo refresh <slug>`; after refreshing, the
agent re-applies its change.

Remote serialization retains the extracted GitHub store’s record and revision
envelopes, artifact-version references and stamp ledger within the
artifacts-and-stamps boundary. A mutation spanning several todos is reflected
per item, not atomically.

A record comment is applied only when GitHub reports its author's association
with the repository as owner, member or collaborator; a record comment by
anyone else is logged and never applied.

Inbound approvals and change requests are accepted only from logins in the
project layer’s `github.approvers` with write access to the repository. An
approver comments on the todo's issue:

- `/instrukt approve <prd|task> <digest-prefix>` — applied by the next pull or
  tick only when the prefix matches the current evidence digest of that gate,
  with GitHub attribution. On a mismatch the issue is answered that the
  evidence changed and must be approved again, and the record is unchanged.
- `/instrukt request-changes <prd|task>` followed by the reason — needs no
  digest.

A comment from anyone else changes no work record and is answered on the
issue.

Reflection retains unfinished work across retries. Rate limiting defers
reflection until the provider permits it; it does not lose local work. Issue,
artifact, pull-request and optional board progress are recoverable without
claiming that a failed request had no remote effect. A retry reconciles against
the remote record before creating duplicate evidence.

### Use cases

#### UC-GR1: Continue recording work during a GitHub outage

```gherkin
  Given a developer has a todo available on their computer
  And GitHub is unreachable
  When they record progress on that todo
  Then the accepted progress remains available locally
  And its GitHub reflection remains pending
  But when no GitHub credentials are available on their computer
  Then the progress is still recorded locally
  And its reflection is reported as pending for lack of credentials
  But when GitHub is limiting their requests
  Then the reflection waits and completes once GitHub accepts requests again
```

#### UC-GR2: Inspect a delivery's evidence on GitHub

```gherkin
  Given a developer's delivered todo has been reflected to GitHub
  When they inspect its issue and linked implementation review
  Then they can follow the intent, retained artifact versions and recorded decisions
  And identify the commit that delivered the work
```

#### UC-GR3: A developer whose todo changed elsewhere is told to refresh first

```gherkin
  Given a developer's todo was changed on GitHub since their computer last read it
  When they record progress on that todo
  Then the change is refused and they are told how to refresh the todo
  And after refreshing they can record their progress against the current todo
```

#### UC-GR4: Records posted by someone without write access never change a developer's work

```gherkin
  Given a developer's project is public and its todos are reflected to GitHub
  When someone without write access to the repository posts what looks like a record update on a todo
  Then the developer's work record is unchanged by it
  And records posted by people with write access are still applied
```

#### UC-GR5: A developer follows their work on a project board

```gherkin
  Given a developer's project reflects its todos to GitHub
  And the developer has chosen a project board for that work
  When their todos are reflected
  Then each todo appears on that board with its current record status
  And the board's own columns and fields are left as they were
  When the developer chooses a different board
  Then their work appears on the new board with its current record status
```

## Known caveats

- instrukt does not create the board project, its views or its readme; the
  predecessor's board setup is out of scope.
- Records written by the predecessor's GitHub store are not read; the envelope
  format is new and carries no compatibility with it.
- A GitHub App bot posts with association `NONE`, so a record it wrote would be
  ignored; instrukt never writes records as an App.
- GitHub reconstruction is limited to the artifact versions the provider
  retains and serves. An older version retained only locally is not promised
  on a replacement computer. Native artifact availability and accepted
  content size also bound remote reflection; failure leaves the local evidence
  and pending reflection visible.
