---
description: The local work record reflected one way to required GitHub issues, boards, native artifacts and live pull requests for people working with agents, with approvals and holds read back from people on GitHub; todos need the project's code on GitHub.
visibility: public
---
# GitHub Record — Spec

## What it is

A developer's work record lives on their computer, and GitHub is where people
and agents meet that work. Each todo shows on GitHub as an issue, a card on the
project board, its readable artifacts and, while it is built and reviewed, a
pull request. People discuss the work there, approve or request changes, and
hold it. Work accepted locally remains available when GitHub is unreachable.
A collaborator reads the durable artifacts and decisions without needing the
conversation that produced them.

## Canonical fields

### The local record and its reflection

The work record is the work tables of this computer's state database
(`project/instrukt/spec/feature/work/state-database`). It is the one authority
for the todos it holds: reads and writes of a todo use it, and a write commits
there. A todo enters it when it is created here.

Reflection carries the record one way, to GitHub. A committed write is
reflected when GitHub is reachable; failure to reflect does not undo it. The
record distinguishes reflection still pending from reflection GitHub
confirmed; local success does not claim remote completion. Nothing read from
GitHub replaces the record or any part of it. What instrukt takes from GitHub
are the controls people exercise there (Approvals and change requests; GitHub
hold label and output labels), each applied to the record as that control's
own change.

GitHub is required, rather than a selectable surface: it is where the people
working on a todo find it, discuss it and decide on it.

### Reading GitHub

A computer reads GitHub only for the todos in its record. No listing ever
fetches, and no command has a `--refresh` flag.

On each runner tick, each computer reads, with conditional requests, the
issues of the todos in its record that are neither delivered nor withdrawn,
so that what a person does on GitHub (an approval, a change request, a hold)
reaches the todo whether or not it is claimed. An issue GitHub reports
unchanged is not read further. Reading an issue whose conversation or label
history has grown costs only what is new since the last read: the record
keeps how far it has read the issue, and never moves that position past a
control it has not yet applied or answered. A lost position costs one
complete read of the issue and applies no control twice. A list
read follows pages up to its page limit; reaching it with a full last page is
stated in the command's report as an incomplete read, never passed off as the
whole list.

Beyond that, a todo's issue is read with a conditional request on
`instrukt todo show <slug>` and before any command acts on the todo. A read
that cannot reach GitHub does not block the command: it proceeds on the
record, says so, and the next read applies what it missed.

### Issues and visible work

Every todo, including a task, an iceboxed todo and a structural holder, is
reflected as an issue in its project's GitHub repository. The issue shows the
current intent, kind where applicable, the record's status (active, iceboxed,
held, delivered or withdrawn; never a workflow phase), hold or icebox state,
group relationships and prerequisites, and beside each open approval gate a
short prefix of the current evidence digest. Delivery closes it as completed and
names the delivery evidence; removal closes it as withdrawn; restoration
reopens it.

The issue's conversation belongs to the people working on the todo. It holds
their comments, their `/instrukt` commands and instrukt's short replies to
those commands. Reflection adds no comment that carries the record, at any
revision. Comments posted before this contract held stay as they are.

The GitHub project board and its views are the developer's cockpit. With todos
enabled, instrukt sets them up and repairs them automatically, without an opt-in.
The developer may name the board in the project layer (`board: <owner>/<number>`)
through `instrukt config`. Otherwise instrukt sets up or finds the project's board
and records its reference in the machine's work record; reflection never writes
the committed project layer
(`project/instrukt/spec/feature/configuration/owned-files`). An absent
reference calls for setup, never silently disables the cockpit. Repeated setup,
on any computer, reconciles existing resources instead of creating duplicate
boards or views.

A board instrukt creates for a public repository is made public when it is
created; the board of any other repository stays private. Whether the
repository is public is read from GitHub before the board is created; while
GitHub's answer does not establish it, no board is created and setup stays
pending. instrukt sets a board's visibility only at its creation: it never
changes the visibility of a board it found or that the developer named, and
later setup never changes it again, so a maintainer who makes the board private
keeps it private. When GitHub declines to make the new board public, for
example because the organization lets only its owners change project
visibility, the board stays private, setup and reflection continue, and the
operator is told that a project admin or organization owner can make it public
in the project's settings. Any other failure of that request is a setup failure
like any other: setup stays pending, and the retry finds the board and leaves
its visibility as it is.

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

Reflection publishes a repository todo's work branch while the todo is built
and reviewed: from the branch its build start records until the todo is
delivered or a withdrawal of it begins. Each runner tick's flush compares the
branch's local head with the head its last publication left on the remote the
default branch tracks, and pushes the branch there when the local head has
moved; a command's own flush publishes nothing, so no command waits on the
remote. A publication only ever
fast-forwards the remote branch; it never forces it. A remote branch that
moved where the local one cannot fast-forward it is reported with the todo
as the reason its publication is outstanding, and is left as it is. An
unreachable remote leaves the publication pending for the next flush; it
delays the publication and never the work. Commits land on the branch as
before: the builder's and the lifecycle's own pushes are unchanged,
and a push of a head the remote already holds changes nothing.

Repository implementation has a linked pull request during its build and
review path, opened as soon as the todo's branch is on GitHub; while the
branch is not available on GitHub, creation remains pending. The pull request
carries the commits under review and each recorded build-review verdict as a
native review of the exact reviewed commit, written by the reviewing role's
App (`project/instrukt/spec/feature/work/github-apps`). A verdict is
reflected once its commit and its report are on GitHub: the commit on the
published branch and the report as a saved version of the `review-findings`
artifact.

A review is written for a person. Its body states the verdict and what the
reviewer found, readable in place, and links the todo's `review-findings`
artifact on the issue. A report longer than a review body holds is carried up
to that length and marked as shortened; the complete report is read in the
linked artifact, whose saved versions include the reviewed one.

Delivery closes the pull request with the delivered commit. The integrator
owns merging and the integration gate; pull-request state alone never proves
delivery.

### Artifacts and stamps

Artifacts are the published work evidence. Each artifact version the record
holds is reflected as a saved version of a native issue artifact, readable on
the issue. Stamps stay in the record; what people decide on GitHub is visible
there as their own comments and labels, and a build-review verdict as its
review on the pull request. Conversational turns, approval questions and other
back-and-forth remain in the configured runtime. Reflection bookkeeping and
the runner's state stay in the state database. Neither the work record nor
any temporary work state is committed to the project's repository.
An evidence reference may identify the originating conversation without
copying it into GitHub.

An artifact's identity on GitHub is its recorded provider address, not its
display name. A saved version is not rewritten to replace history.
Implementation plans and task plans use the provider's plan capability; other
artifacts use its document capability. Saved versions on GitHub show the
record's versions and are never read back as evidence: a similarly named
artifact or an unrelated edit on GitHub changes nothing in the record.

An artifact's saved versions are read completely, across as many pages as
GitHub serves them in. Reflecting a new version costs the same whatever the
artifact's history: the upload is confirmed against the version GitHub saved
for it, without reading the content of earlier versions.

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
of each build, the delivered commit and the issue live in the same
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
interactive paths and ticks alike; tokens remain in memory. Issues, bugs and
board updates use the project bot; role-driven pull requests and reviews use
their role's App. Process environment and `gh auth token` supply no alternate
credential source. With unavailable credentials, reflection remains visibly
pending and the local record continues. Identity/issuance failures retain
their secret-safe cause and never select a human fallback.

A per-computer reflection lock `~/.instrukt/state/reflection.lock` serializes
reflection; no separate writer process runs. Every runner tick flushes
pending reflection under that lock, held across GitHub requests. A command
that writes the record and finds the lock free flushes pending reflection
under it before it returns. A command that finds the lock held by another
process never waits on it: it returns as soon as its local write is
committed, reports its reflection as pending, and leaves what it did not
reflect to the next flush that acquires the lock. The
separate writer lock guards only local
state-database write transactions and is released before each network wait. Reflection
takes it briefly for each local commit of what it learned. Lock order is always
reflection → writer; commands finish their local write and release the writer
lock before trying the reflection lock. Local readers take neither lock.
Confirmation records the revision actually reflected; a newer local revision
remains pending.

Reflection writes what the record holds. It does not check GitHub for a newer
version of the record before writing, and nothing on GitHub makes a todo
refuse commands. A mutation spanning several todos commits locally in one
transaction and is reflected per item, not atomically.

Inbound approvals and change requests are accepted only from logins in the
project layer’s `github.approvers` with write access to the repository. An
approver comments on the todo's issue:

- `/instrukt approve <prd|task> <digest-prefix>` — applied by the next read
  of the issue only when the prefix matches the current evidence digest of that gate,
  with GitHub attribution. On a mismatch the issue is answered that the
  evidence changed and must be approved again, and the record is unchanged.
- `/instrukt request-changes <prd|task>` followed by the reason — needs no
  digest.

A comment from anyone else changes no work record and is answered on the
issue.

An applied approval or request-changes decision and its pending attention
commit together. The notice carries the decision, gate, reason when given,
attribution and bound evidence. Delivery targets the session holding the
claim and retries as specified by
`project/instrukt/spec/feature/runner/session-continuity`.
Re-reading an already applied comment neither reapplies the decision nor
creates another notice. The decision stamp retains its source comment
identity in the record. Its confirmation reply is retained with acceptance and
retried separately. A source-comment marker reconciles a reply whose posting
acknowledgement was lost, so confirmation retry changes no work fact.

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

Reading the issue reads attributable label changes and applies each accepted
change once, in source order, retaining its GitHub action identity. Applied
action identities are kept in the record. Retrying a read cannot reapply a
processed action against a later manual hold. instrukt's own reflection
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
what GitHub already shows before creating a duplicate issue, artifact version,
pull request or review.

Content GitHub refuses as invalid is a refusal of that content, where an
unreachable or rate-limiting GitHub is a wait. The refusal is kept with the
validation detail GitHub gave and reported with the todo as the reason its
reflection is outstanding. The same content is not sent again: a refused
request is sent again only once what it sends, or the published branch head
it concerns, has changed. The todo's other reflection continues, the local
record continues and other todos reflect independently. A refused artifact
version is followed by the artifact's next version as usual.

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
  But when GitHub refuses the reflected content as invalid
  Then the progress is still recorded locally
  And the refusal is reported with the reason GitHub gave for it
  But when another process on their computer is reflecting
  Then the command returns as soon as the progress is recorded locally
  And its reflection remains pending until the next reflection on their computer reflects it
```

### UC-GR2: Inspect a delivery's evidence on GitHub

```gherkin
  Given a developer's delivered todo has been reflected to GitHub
  When they inspect its issue and linked implementation review
  Then they can follow the intent, retained artifact versions and recorded decisions
  And identify the commit that delivered the work
```

### UC-GR3: A developer whose todo changed elsewhere is told to refresh first

### UC-GR4: Records posted by someone without write access never change a developer's work

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

### UC-GR8: Record a GitHub approval decision

```gherkin
Given a todo has current approval-gate evidence
When an authorized approver submits an admissible approval or request-changes comment
Then the decision records GitHub attribution and current evidence
And replaying the comment creates no second decision or notice
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

### UC-GR11: A developer's public project gets a public board

```gherkin
Given a developer's project is public and reflects its todos to GitHub
When their todos are reflected without a board already set up
Then the board instrukt creates is public
When a maintainer makes that board private
Then it stays private as the project's todos are reflected again, from this computer and from another
And a board the developer then names in the project layer keeps its visibility
```

### UC-GR12: A developer follows their work on a new board GitHub would not make public

```gherkin
Given a developer's project is public and GitHub does not let instrukt change the visibility of projects
When their todos are reflected without a board already set up
Then instrukt sets up the board and it stays private
And their todos appear on it with their record status
```

### UC-GR13: Discuss a todo on its issue

```gherkin
Given a developer's todo is reflected to GitHub while agents keep recording progress on it
When the developer and collaborators discuss the todo on its issue
Then the conversation holds their comments, their instrukt commands and the replies to those commands
And the recorded progress adds no comment to it
```

### UC-GR14: Read a build review on the todo's pull request

```gherkin
Given a developer's repository todo has its branch on GitHub while it is built and reviewed
When an independent reviewer records a verdict on a commit of that branch
Then the todo's pull request is open and shows the reviewer bot's review of that commit
And the review states the verdict and what the reviewer found, readable in place
And it links the todo's review findings artifact
```

### UC-GR16: Follow a todo's work on its pull request while agents build it

```gherkin
Given a developer's repository todo is being built and reviewed on their computer
When a builder lands a commit on the todo's branch
Then the commit is on the todo's branch on GitHub
And the todo's pull request is open and shows it
But when GitHub cannot be reached
Then the work on the todo continues
And the commit reaches GitHub once it can be reached again
But when the branch on GitHub has moved where the todo's branch cannot follow it
Then the branch on GitHub is left as it is
And the developer is told why the todo's commits are not on GitHub
```

## Known caveats

- A new board is made public in a request that follows its creation. When that
  request ends without GitHub's answer, or GitHub is limiting requests at that
  moment, the board keeps whichever visibility GitHub gave it, which instrukt
  does not know, and nothing changes it later: a later pass cannot tell an
  interrupted request from a maintainer's choice. The board's visibility is
  read and changed in the project's settings.

- Code hosted anywhere but GitHub is not supported for todos.
- Whether `github.repository` names the repository the remote reaches is
  taken as written; instrukt does not check it.
- GitHub holds no copy of the work record. A todo is worked on the computer
  whose record holds it, and a record lost with its state database is not
  recovered from GitHub; preserving the state database is
  `project/instrukt/spec/feature/work/state-database`'s.
- Native artifact availability and accepted content size bound reflection;
  failure leaves the local evidence and pending reflection visible.

## See Also
- [project/instrukt/spec/domain/work](../../domain/work.md)
