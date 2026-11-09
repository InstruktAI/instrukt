---
description: Local-leading work records reflected to required GitHub issues with versioned native artifacts and revision checks; todos need the project's code on GitHub, and GitHub is read only for the todos this computer has an interest in.
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

The local record is the work tables of this computer's state database
(`project/instrukt/spec/feature/work/state-database`): the local store
in front of the GitHub reflection. A *local item* is a todo present in it. An
item enters the local record when dispatched here or when a command changes
it here. Reads and writes of local items use that local record. A write
commits there and is reflected to GitHub when reachable. Failure to reflect
does not undo a locally accepted write. The record distinguishes pending
reflection from a confirmed remote revision; local success does not claim
remote completion.

GitHub is required, rather than a selectable storage backend: it is the
reflected store from which work is recovered and through which it moves
between computers. What instrukt reads from GitHub is committed to the state
database before readers consume it (`Interest-driven refresh`). Rebuilding
from GitHub recovers reflected work, subject to the artifact history GitHub
still serves. Unreflected local writes have no remote recovery guarantee.

### Interest-driven refresh

A computer reads GitHub only for the todos it has an interest in. No listing
ever fetches, and no command has a `--refresh` flag.

On each runner tick, each computer polls, with conditional requests:

- the todos it holds claims on, so that what a human does on GitHub (an
  approval, a change request, a record comment) reaches the worker holding
  the claim;
- one pickup listing per project whose effective configuration enables
  `todos`: the project's todo issues, whose result replaces that project's
  listing copies;
- the prerequisites its own claimed work is blocked on.

Beyond that, a todo is refreshed implicitly, one at a time, with a
conditional request:

- on `instrukt todo show <slug>`;
- when its claim moves to this computer, before the claim is committed;
- before any command acts on it.

A refreshed todo that is not a local item replaces its listing copy and does
not become a local item. A refresh that cannot reach GitHub does not block
the command: it proceeds on what the state database holds, says so, and the
revision check at reflection catches any change it missed.

Listings, `instrukt todo list` among them, read only the state database. The
todos this computer holds claims on are shown from the local record. Every
other todo is shown as its last known copy, the local record's or the pickup
listing's, whichever was read later (`todos.refreshed_at` and `listing_copies.copied_at` respectively), labelled with the computer its claim
names, or unclaimed, and how old the copy is.

### Issues and visible work

Every todo, including a task, an iceboxed todo and a structural holder, is
reflected as an issue in its project's GitHub repository. The issue shows the
current intent, kind where applicable, the record's status (active, iceboxed,
held, delivered or withdrawn; never a workflow phase), hold or icebox state,
group relationships and prerequisites, and beside each open approval gate a
short prefix of the current evidence digest. Delivery closes it as completed and
names the delivery evidence; removal closes it as withdrawn; restoration
reopens it.

The GitHub project board and its views are the developer's cockpit. With todos
enabled, instrukt sets them up and repairs them automatically, without an opt-in.
The project layer records the board reference (`<owner>/<number>`); an absent
reference calls for setup, never silently disables the cockpit. Repeated setup
reconciles existing resources instead of creating duplicate boards or views.

instrukt adds every reflected issue and maintains its owned single-select
`Record status` field and required options (active, iceboxed, held, delivered,
withdrawn). Its owned cockpit views are:

- `Board`: board layout, no filter, columns grouped by `Record status` (one
  column per option).
- `Needs you`: table layout, filter `needs-you:Yes`.

The owned single-select `Needs you` field has exactly one option: `Yes`, red,
with description `Waiting on a person; instrukt will not move it.` An item has
this option when its record status is held, or a current approval gate waits
for a human under `approvals.prd` or `approvals.task`. A PRD reaches that
moment only after its current independent review approves its current target
and report; drafting, revision and review work remain unmarked. Evidence or
review invalidation returns the work to agents and clears the marker unless
another person wait remains. Peer-only approval gates
do not set it. Satisfying the gate, releasing the hold, or changing the gate to
peer approval clears the marker when no other human wait remains. The marker
is output, never an inbound command or a workflow fact.

Setup creates missing fields and views and repairs owned layout, filter,
column-field and option drift. Repeated setup reuses matching resources.
Repairing a wrong column field can replace the owned `Board` view and change
its address.
Unrelated user fields, views and GitHub's built-in project workflows remain
untouched. Moving a card establishes no workflow fact. Setup, view repair and
board reflection failures remain visible and retryable independently of issue and
local-record writes; missing GitHub permissions are a reported setup failure,
never an implicit opt-out.

Repository implementation has a linked pull request during its build and
review path. It carries the reviewed commits and recorded verdicts. If the
branch is not available on GitHub, creation remains pending. Delivery closes
the pull request with the delivered commit. The integrator owns merging and
the integration gate; pull-request state alone never proves delivery.

### Artifacts and stamps

Artifacts and stamps are the published work evidence. Conversational turns,
approval questions and other back-and-forth remain in the configured runtime.
Reflection bookkeeping and the runner's state stay in the state database and
are never reflected. Neither the work record nor any temporary work state is
committed to the project's repository.
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
`github.repository: owner/repo` overrides it, for a remote that reaches that
GitHub repository through an address that does not read as one, such as an
SSH host alias.

**Code on GitHub.** Todos need the project's code on GitHub: the pull request
of each build, the delivered commit and the record live in the same
repository. The code is on GitHub when the remote the default branch tracks
is a repository on `github.api_url`'s host, or when `github.repository` names
the repository that remote reaches. Turning todos on for a project whose code
is not on GitHub is refused with a plain reason, that todos need the
project's code on GitHub, naming the remote and `github.repository`; the
configuration is unchanged. When todos are on from the global layer, such a
project's todo commands refuse with the same reason and the runner picks up
nothing for it. `instrukt status` reports whether the repository's code is on
GitHub, so the instrukt skill says so before switching todos on.

**Credentials and actors.** The GitHub boundary issues installation tokens for
configured Apps under `project/instrukt/spec/feature/work/github-apps`. App keys
resolve from the one owner-only machine-local `~/.instrukt/secrets.env` on
interactive paths and ticks alike; tokens remain in memory. Shared records,
issues, bugs and board updates use the project bot; role-driven pull requests
and reviews use their role's App. Process environment and `gh auth token` supply
no alternate credential source. With unavailable credentials, reflection remains
visibly pending and the local record continues. Identity/issuance failures retain
their secret-safe cause and never select a human fallback.

A per-computer reflection lock `~/.instrukt/state/reflection.lock` serializes
reflection; no separate writer process runs. Every command that writes the
record and every runner tick flushes pending reflection under that lock,
held across GitHub requests. The separate writer lock guards only local
state-database write transactions and is released before each network wait. Reflection
takes it briefly for each local commit of what it learned. Lock order is always
reflection → writer; commands finish their local write and release the writer
lock before acquiring the reflection lock. Local readers take neither lock.
Confirmation records the revision actually reflected; a newer local revision
remains pending.

Remote writes check the item's known remote revision before replacing it.
A newer remote revision is a conflict, not permission to overwrite it with a
stale local snapshot. The local change
is not reflected and the item is marked conflicted. Commands on that item
refuse with the door `instrukt todo refresh <slug>`; after refreshing, the
agent re-applies its change.

Remote serialization retains the extracted GitHub store’s record and revision
envelopes, artifact-version references and stamp ledger within the
artifacts-and-stamps boundary. A mutation spanning several todos commits
locally in one transaction and is reflected per item, not atomically.

A record comment is applied when GitHub reports its author's association with
the repository as owner, member or collaborator, or when its verified stable
actor matches a configured App bot under
`project/instrukt/spec/feature/work/github-apps`. Configured bots are trusted
for record restoration even with association `NONE`. Any other author is logged
and never applied. Envelope trust does not grant human approval or hold-label
authority; those controls retain their actor checks below.

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

An applied approval or request-changes decision wakes the todo's current
claim-holding orchestrator if that session is alive, with the as-soon-as-possible
intent. The notice carries the decision, gate, reason when given, attribution
and bound evidence. The decision and pending attention commit before runtime
I/O; failed observation or delivery retains its cause and retries under
`project/instrukt/spec/feature/runner/session-continuity`. With no live
orchestrator it is only a state update: next pickup or a deliberate start
continues from it. No approval event or second stamp-driven wake is emitted.
The claim-hosting computer recovers attention for imported decisions exactly once locally. A foreign computer never classifies a claimant absent from its local runtime as gone. Re-reading an already applied comment neither reapplies the decision nor creates another notice on that computer. The decision stamp retains its source comment identity
through reflection, claim movement and local database reconstruction. Its confirmation reply is retained with acceptance and
retried separately. A source-comment marker reconciles a reply whose posting
acknowledgement was lost, so confirmation retry changes no work fact.

### Record schema compatibility

Every GitHub record/revision envelope declares its record schema version.
Every reader checks it before interpreting or applying that envelope: interest
refresh, pickup listings, reconstruction and mutation pull-through use the same
compatibility path. Local database and configuration versions remain separate.

A version newer than this binary understands is refused before any dependent
mutation: no local record replacement, stamp/artifact append, claim change,
reflection, acknowledgement comment or remote rewrite proceeds against it.
The refusal names the todo, observed and supported versions, and the upgrade
door. The remote envelope and existing local evidence remain intact; a listing
reports the incompatible item rather than silently dropping it. Unrelated work
can continue. A known incompatible record is not treated as an ordinary offline
refresh failure permitting a stale write.

Native driver placement evidence carries its captured display title. Supported
older record conversion preserves retained title evidence or recovers the old
producer's readable title from the binding's role and original phase branch before
deserializing the current work record. It uses no current todo slug, treats no
omission as proof of a key-only native title, and refuses missing recovery facts.
The whole-key native-title allowance remains with session continuity.

A supported older version is converted to the current in-memory record before
consumption or mutation. Conversion preserves durable identity, relationships,
artifact-version references, digests, stamps, actor attribution and revision
history. Missing conversion or invalid content refuses with the source intact;
it never fabricates evidence or resets the todo. Conversion is not a fresh
human decision or workflow fact. Persisting a converted record uses the normal
local transaction and remote revision-checked reflection; old retained revisions
and artifact content are not rewritten. Re-reading a converted record converges
without duplicating decisions. Offline operation keeps the existing local-leading
contract; it cannot claim to have checked an unavailable remote version.

### GitHub hold label and output labels

`hold` is the one inbound control label. Adding it on the todo's issue by a
collaborator with repository write access sets the todo's dispatch hold, with
generated reason `held from GitHub by <login>`, GitHub actor, source and time.
Removing it by a collaborator with write access releases the current dispatch
hold and attributes that release to the removing login. Neither action changes
the claim, artifacts, prerequisites or work already running. Structural holders
keep their existing no-hold rule. Label actions from a login without write
access change no work record; unlike approval commands, authorized hold changes
need no membership in `github.approvers`.

Interest-driven refresh reads attributable label changes and applies each
accepted change once, in source order, retaining its GitHub action identity.
Applied action identities travel in the reflected record and are reconciled on every computer and database reconstruction. Retrying a refresh cannot reapply a processed action against a later manual hold. instrukt's own reflection
writes are reconciled as output, not interpreted as fresh human commands.
A label snapshot lacking its action and author is insufficient to attribute a
new hold or release. Inability to verify the actor or read changes is reported
and leaves the record unchanged for that action.

Every other label instrukt writes is output only. Reflection restores those
labels from record status and never reads a collaborator's label edit as work
state. Labels outside instrukt's output set remain collaborator-owned. Board
lanes and `Record status` reflection retain the existing contract; moving a
card changes no work fact. Hold semantics belong to
`project/instrukt/spec/feature/work/dependencies-and-holds`.

Reflection retains unfinished work across retries. Rate limiting defers
reflection until the provider permits it; it does not lose local work. Issue,
artifact, pull-request and board progress are recoverable without
claiming that a failed request had no remote effect. A retry reconciles against
the remote record before creating duplicate evidence.

## Use cases

### UC-GR1: Continue recording work during a GitHub outage

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

### UC-GR2: Inspect a delivery's evidence on GitHub

```gherkin
  Given a developer's delivered todo has been reflected to GitHub
  When they inspect its issue and linked implementation review
  Then they can follow the intent, retained artifact versions and recorded decisions
  And identify the commit that delivered the work
```

### UC-GR3: A developer whose todo changed elsewhere is told to refresh first

```gherkin
  Given a developer recorded progress on a todo while another computer recorded a change to the same todo
  And the other computer's change reached GitHub first
  When the developer's progress is reflected
  Then it does not overwrite the other change, and they are told how to refresh the todo
  And after refreshing they can record their progress against the current todo
```

### UC-GR4: Records posted by someone without write access never change a developer's work

```gherkin
  Given a developer's project is public and its todos are reflected to GitHub
  When someone without write access to the repository posts what looks like a record update on a todo
  Then the developer's work record is unchanged by it
  And records posted by people with write access and configured verified bots are still applied
```

### UC-GR5: A developer follows their work on a project board

```gherkin
  Given a developer's project reflects its todos to GitHub
  When their todos are reflected without a board already set up
  Then instrukt sets up the project board and its views automatically
  And the Board view uses board layout with columns by Record status and no filter
  And the Needs you view uses table layout filtered by needs-you:Yes
  And each todo appears with its current record status
  And the Needs you field has only the red Yes option with its documented description
  When an unheld todo is drafting, being revised or awaiting independent review
  Then its Needs you marker is absent
  When a todo is held or has reached its current human approval gate under the project's approvals configuration
  Then its Needs you marker is set
  When that wait ends or only a peer approval is outstanding
  Then the marker is cleared unless another human wait remains
  When an owned cockpit field or view is missing or its options, layout, filter or column field drift
  Then instrukt repairs it without requiring opt-in or leaving duplicate owned resources
  And unrelated user fields, views and built-in project workflows are preserved
```

### UC-GR6: A developer whose code is not on GitHub is told todos need it there

```gherkin
  Given a developer's project keeps its code on a host other than GitHub
  When they switch todos on for that project
  Then the change is refused, telling them in plain words that todos need the project's code on GitHub
  And their configuration is unchanged
```

### UC-GR7: A todo whose claim moves to a developer's computer is current when work starts

```gherkin
  Given a collaborator changed a todo on GitHub while another computer held it
  When the todo's claim moves to the developer's computer
  Then the work starts from the todo as GitHub holds it now
  And each later command on that todo acts on its current state
```

### UC-GR8: Continue preparation after a GitHub decision

```gherkin
Given a developer has a current reviewed approval gate on a todo's GitHub issue
When an authorized approver submits an admissible approval or request-changes comment
Then the decision is recorded with GitHub attribution and current evidence
And its live claim-holding orchestrator receives the decision as soon as its runtime permits
But when there is no live orchestrator
Then only the record changes and the next pickup or deliberate start continues from it
And replaying that comment creates no second decision notice
```

### UC-GR9: Control a todo's hold from its GitHub issue

```gherkin
Given a developer follows a todo through its GitHub issue
When a collaborator with write access adds the hold label
Then the todo is held with reason "held from GitHub by <login>" attributed to that collaborator
When a collaborator with write access removes that label
Then the dispatch hold is released with the removing login recorded
And neither operation changes the todo's evidence or claim
But when another instrukt-written label is edited or the board card is moved
Then no work state is changed and reflection restores the output labels
But when the actor lacks write access
Then their hold-label action changes no work record
```

### UC-GR10: Recover work across supported record schema upgrades

```gherkin
Given a developer restores reflected work written with an older supported record schema
When instrukt reads its record and revision envelopes
Then it converts them before consumption while preserving the retained evidence and attribution
But a newer schema refuses before any dependent mutation and names the upgrade door
And the remote record and existing local evidence remain intact
```

## Known caveats

- Code hosted anywhere but GitHub is not supported for todos.
- Whether `github.repository` names the repository the remote reaches is
  taken as written; instrukt does not check it.
- A listing's copy of a todo held elsewhere is as old as the last pickup
  listing; the label says how old.
- Records written by the predecessor's GitHub store are not read; the envelope
  format is new and carries no compatibility with it.
- GitHub reconstruction is limited to the artifact versions the provider
  retains and serves. An older version retained only locally is not promised
  on a replacement computer. Native artifact availability and accepted
  content size also bound remote reflection; failure leaves the local evidence
  and pending reflection visible.

## See Also
- [project/instrukt/spec/domain/work](../../domain/work.md)
