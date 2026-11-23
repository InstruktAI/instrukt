---
description: Role-selected GitHub App bot identities, protected keys and installation tokens for visible bot authorship, independent native reviews and trusted record recovery.
---
# GitHub Apps — Spec

## What it is

instrukt writes to GitHub as configured GitHub Apps. The bot is visible on
records, issues, bugs, board updates, pull requests and reviews. Authoring and
review use separate bots, so GitHub's native prohibition on author self-approval
does not prevent independent review. Account lineage is distinct from who did
the work (`general/concept/lineage-vs-authorship`).

## Canonical fields

### App inventory and role selection

The machine-global `instrukt.yaml` declares `github.apps`, a map of public App
profiles, and `github.project_app`, the project-identity profile reference.
Each profile records `app_id`, `slug`, `bot: {id, node_id, login}` (the stable
bot account identity), and `private_key` (a protected secret reference). The key
reference is `${NAME}` into the protected machine-local secrets file, never
the key itself, a repository-selected path or an environment fallback.
Configuration validates shapes, distinct App/bot identities and all references;
the GitHub boundary verifies the configured actor against GitHub.

The actual recorded session role selects `roles.<role>.github_app` in
`~/.instrukt/agents.yaml`. An omitted mapping uses `github.project_app`,
including an unknown role resolved through `default`. No per-task model override,
command argument or profile order selects the GitHub actor. Background record,
issue, bug and board maintenance uses the project identity. Role-driven pull
request authoring and reviews use the selected role identity. Human work
performed directly on GitHub keeps its human identity; instrukt's shared-record
maintenance still uses the project App. This contract changes no human login/
account association or runtime permission mode.

| Profile reference | Shipped identity | Responsibility |
| --- | --- | --- |
| `project` | `instruktai[bot]` | Shared records, issues, bugs and the board; roles without a specific mapping. |
| `author` | `instruktai-author[bot]` | Authoring roles, their pull requests and bot-authored commits. |
| `reviewer` | `instruktai-reviewer[bot]` | Reviewer roles and their native GitHub reviews. |

The shipped role compositions live in
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. An
independent reviewer uses a different App/bot from the pull request author;
using a different session of the same App does not satisfy GitHub's native
restriction. Review stamps retain their own independence and exact-evidence
requirements; different bots alone do not prove review occurred.

### Configuration skill and installation

The configuration skill reuses the three existing TeleClaude Apps on the
installation that already has them, verifying their configured identity and
repository installation instead of creating duplicates. On other installations
it guides creation and installation of the three Apps and records their actual
public identities; globally unique slugs can differ from the shipped names.
It writes private keys only through `instrukt secrets set`, into
`~/.instrukt/secrets.env` under the owned-files contract. No key or token is
carried in a configuration report, repo file or assignment brief.

Setup verifies the App installation reaches the project's repository and has
the permissions required by the operation: records/issues/artifacts and board
maintenance, code/pull-request authoring, or reviews/check observation as
applicable. Missing installation, permissions or an API capability remains an
explicit incomplete setup with its repair door. No personal token or `gh`
login is silently substituted for the selected App.

### Installation credentials

The shared GitHub boundary resolves the chosen App's key from the protected
file on interactive and scheduled paths alike. It authenticates as that App
with a signed JWT, discovers/verifies its installation for the target
repository, and requests an installation access token with the required
repository and permission scope. It verifies returned identity, repository
access, permissions and usable expiry before use. Tokens expire and are
renewed from the same App identity; renewal is not a switch of actor.
Account/project board operations use the installation's applicable account
permissions rather than assuming a repository token grants board access.

Private keys remain protected-file material. Issued tokens are short-lived
in-memory credentials, never persisted in records, runtime assignment evidence,
git remotes, logs or configuration. An instrukt-owned child needing GitHub
credentials receives only the bounded installation credential for that
operation, never the App key. Raw agent `gh`/git use remains with the runtime
permission and credential integration; this does not add a general secret
export command or a permission gate for those external tools.

Absent keys, an unavailable installation, identity/scope mismatch or failed
issuance leaves the GitHub operation visibly pending/refused with a secret-safe
cause. Rate limits retain their retry boundary. Locally accepted work and its
pending reflection survive; no credential failure is reported as remote success
or as proof that an ambiguous write had no effect. The record's existing
revision and retry-convergence rules remain in force.

### Read-only reflection readiness

Work verifies the project App, protected key, repository installation,
record and account-board permission scopes, expiry and stable bot identities.
It reads repository issues, existing issue artifacts when an issue is available,
and the configured board or its account's project listing. Newer pages containing
only pull requests do not exclude older issues from artifact readiness checks.
Unavailable access or reflection capabilities are unmet findings with the
configuration repair door.
Read-only readiness creates no issues, artifacts, boards, fields or views.
It establishes declared write permissions and available read surfaces; it cannot
prove a future remote write, or artifact access on an empty repository, without
performing a mutation.

### Restore trust and commit authorship

Record restoration accepts an author matching a configured App's verified
stable bot identity even when GitHub reports association `NONE`. A display
login or bot-shaped name alone establishes no trust. Repository owner/member/
collaborator records retain their existing admission. Trust admits record
envelope processing; it bypasses neither schema-version checks, digests,
revision history nor approval attribution. Bot trust grants no human approval
or hold-control authority.

The App boundary supplies verified bot actor facts; commit-author resolution,
precedence and refusal belong to
`project/instrukt/spec/feature/repository/attributed-commit`.

<!-- planned:event-driven-pr-review-gate -->
### Review verdict on the reviewed commit

A verdict of an automatically started pull-request review
(`project/instrukt/spec/feature/runner/pull-request-review`) is published by the
reviewer bot in two places:

| Verdict | Pull-request review | Review status on the reviewed commit |
| --- | --- | --- |
| Approve | Approving review | Success |
| Needs work | Change-requesting review carrying the findings | Failure |

Both follow the verdict's evidence record on the commit (Evidence records on
GitHub). The status is named `instrukt/review`. It names the exact commit that was
reviewed; a later head never inherits it. The reviewer bot is the status's only
publisher; a status of the same name from another identity is not a verdict.
The reviewer identity is the App the `commit-reviewer` role selects.

The verdict is a judgment of the commit; the pull-request review is its
delivery to one pull request. Every eligible pull request whose current head is
that commit receives its own review of the one verdict, and the commit receives
one status. The status belongs to the commit and admits it wherever it is
proposed, so the verdict behind it is a judgment of everything the commit adds
to the canonical branch (Admission of review evidence, Range).

### Review evidence

Work keeps three kinds of review evidence for exact commits that no todo's
record carries. A head verdict and a derived approval are a commit's verdict;
an exemption is not.

| Evidence | Records | Recorded by |
| --- | --- | --- |
| Head verdict | An independent review's outcome for one commit: the commit, the pinned base the review started from, the verdict, the complete report, the reviewing session, the review request it answers when it answers one, and which of the reviewed commits name no authoring session. | The reviewing session, with `instrukt review record`. |
| Exemption | The decision that a commit owes no independent review under `general/policy/commits`, with its stated basis, the session that declared it when one resolves, and whether the commit names an authoring session. | The commit's author, with `instrukt review exempt`. |
| Derived approval | Workflow's finding that an integration candidate's content is covered (`project/instrukt/spec/feature/workflow/integration-gate`), with the canonical head it was derived against and every covering item. | The runner's tick, through work's command contract. No agent command records one. |

| Command | Contract |
| --- | --- |
| `instrukt review record --head <sha> --base <sha> --verdict approve\|needs_work --report <file\|->` | Records a head verdict of the commits after the base up to the head. |
| `instrukt review exempt --commit <sha> --basis <text>` | Records an exemption for a commit the canonical branch does not contain yet. |
| `instrukt review show <sha>` | The commit's evidence: its verdict versions, exemption or derived approval, each with its pinned base, where it was recorded, its evidence record on GitHub and every publication with its state and the cause of an outstanding or refused one. |

The first verdict admitted for a commit settles its review. Every review
request for that commit asks for that one judgment, so a request answered once
the commit is settled records nothing. A later version exists only as a
revision: the session that recorded the current version judges its own findings
again, as when the author disputes them with evidence and no new commit. The
latest version is current and earlier ones are kept as history.

A head verdict is its reviewer's to record. Whether the review was requested,
when it took place and whether the project has opted in decide nothing about
its admission: a review that finished earlier is recorded from its retained
report by the session that reviewed, and counts like any other. Recording needs
no opt-in; publication does (Verdict publication).

### Admission of review evidence

`instrukt review record` admits a verdict only when all of these hold. They are
facts of the reviewed commit and of the calling session's recorded relation to
it, and they bind every caller, a session without a role included
(`project/instrukt/spec/feature/configuration/roles-and-capabilities`).

| Invariant | Rule | The refusal names |
| --- | --- | --- |
| Repository | `--head` is the full id of a commit present in the repository of the project the caller stands in. | The commit and the project. |
| Reviewer | The caller is a session the selected runtime adapter recognises, resolved as for an attributed commit (`project/instrukt/spec/feature/repository/attributed-commit`, Resolution). It is recorded as the reviewing session. | That no session resolves. |
| Request | Read from the runner (`project/instrukt/spec/feature/runner/pull-request-review`, One review per commit). A caller that was handed a review request for exactly this project and commit records that review. Any other caller is refused while the commit has a queued or in-flight request: the review is that request's. A commit with neither is unrequested and its verdict is a one-off review's. | The request, and the session that holds it once it was handed over. |
| Range | The pinned base is the merge base of the head with the project's canonical branch, so a head verdict covers everything its head adds to that branch, whatever branch a pull request at that head targets. A request carries it for a requested review; work computes it from the repository for an unrequested one, as `sdlc/procedure/lifecycle/work/review-commits` pins its candidate. A later version of a settled commit keeps the pinned base of the version it replaces. `--base` states where the review started and is admitted when it is the pinned base or one of that base's ancestors. The caller cannot narrow the range. | The pinned base. |
| Independence | No commit after the pinned base up to the head names the reviewing session in its `Session:` trailer. | The commits that name the caller. The door is a review by a session none of them names. |
| Settlement | The commit has no verdict yet, or the verdict is a revision: the caller is the session that recorded the current version, and its current turn is one that no review request for this commit entered. | The settled verdict and `instrukt review show`; for the session that recorded it, that a revision is recorded in a turn no review request for the commit entered. |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

A request is answered in the turn it entered. A verdict offered in such a turn
for a commit that is already settled answers that request a second time,
whichever session offers it and whichever request it is: the one first handed
over, one emitted after an unconfirmed handover, or one delivery replayed. It
is refused and no version is recorded. Whether the caller's current turn was
entered by a review request for the commit is the runner's answer
(`project/instrukt/spec/feature/runner/pull-request-review`, One review per
commit), asked only for a settled commit and its settling session. While the
runtime cannot give it, the verdict is refused as not yet decidable and the
caller records again.

A commit without a `Session:` trailer names no session, and that is its true
state (`general/concept/lineage-vs-authorship`). Work refuses nothing on that
and reads nothing into it: the verdict records such a commit as naming no
authoring session, and counts like any other verdict. Independence from
whoever wrote such a commit is the reviewing procedure's own duty and is not a
fact work established.

The caller names no pull request and no request. Where a verdict is delivered
follows from which eligible pull requests have the commit as their current
head, as the runner observed them. A request handed over for an earlier head of
a pull request stays recordable after the pull request has moved on; its
verdict is that commit's and no other's.

`instrukt review exempt` admits an exemption for a commit in the project's
repository that the canonical branch does not contain. When the commit names an
authoring session, only that session's exemption is admitted and the refusal
names that session. When the commit names none, the exemption is admitted as
the caller's declaration and recorded with the declaring session, when one
resolves, and with the fact that the commit names no session; work neither
refuses it for that nor supplies an author. An exemption counts in candidate
coverage alone and never gives a commit a review status.

A derived approval is admitted only for the fetched integration head while the
canonical branch does not contain it, and only with the coverage finding it
rests on.

### Evidence records on GitHub

Review evidence belongs to the project, and GitHub carries it between computers
as it carries the work record
(`project/instrukt/spec/feature/work/github-record`, Local-leading
pull-through): evidence is recorded locally first and then reflected as an
evidence record on the commit it is about.

| Evidence | Its evidence record carries |
| --- | --- |
| Head verdict version | The version whole: verdict, pinned base, complete report, reviewing session, request and the commits that name no authoring session. |
| Derived approval | The canonical head it was derived against and every covering item with the reference to its evidence (Verdict publication). |
| Exemption | The basis and who declared it. |

An evidence record is two objects on the commit, both published by the
reviewer identity.

| Part | GitHub object | Holds |
| --- | --- | --- |
| The record | A completed check run named `instrukt/review-record`, concluded neutral | The evidence whole, in the check run's output. |
| Its anchor | A commit status named `instrukt/review-anchor`, with state success | In its description, the digest of the record's whole content. |

What GitHub returns for a record is evidence only through its anchor. Work
reads the reviewer bot's anchors on the commit, recognised by its verified bot
identity, and accepts the record whose whole content has the digest an anchor
names. A record without an anchor, one whose content differs from its anchor
and one with a part missing supply no evidence, whoever GitHub names as their
creator. The anchor is a commit status because GitHub's status interface
creates and lists statuses, offers nothing that changes or removes one, and
names the creator of each. The record is a check run because only a GitHub App
can write one, under a permission readiness verifies (Readiness for automatic
review), where a comment's text can be changed by anyone who may write to the
repository. A version of an artifact counts the same way in the work record:
only when a committed revision names its digest
(`project/instrukt/spec/feature/work/github-record`, Artifacts and stamps).

A record declares its record schema version and is read under the
compatibility rule of `project/instrukt/spec/feature/work/github-record`
(Record schema compatibility). One that does not fit a single check run is
carried in numbered parts under one anchor whose digest covers them all. A
record is never rewritten: a later version is a later record with a later
anchor.

The newest anchor on a commit names its current record. Every earlier record
is history from then on, also when the record the newest anchor names cannot
be read whole: the commit then has no recognised evidence, and an earlier
verdict does not come back.

The record is published first and its anchor second. The review status and the
pull-request reviews are written only once the anchor is confirmed on GitHub,
so whatever GitHub shows of a verdict has its verifiable evidence beside it.
The review status alone is never evidence in instrukt: a commit that carries
the status and no anchored record has no recognised verdict.

Which evidence is current for a commit is resolved in this order, and every
reader of review evidence asks work for this answer:

| Work holds for the commit | Current evidence |
| --- | --- |
| A version recorded on this computer that is not yet reflected, or that is the record GitHub's newest anchor names | That version. What GitHub still shows of an earlier version decides nothing: an approving status that a later needs-work version replaced approves nothing. |
| A version recorded on this computer that GitHub moved past before it was reflected: GitHub carries a newer anchor, written by another computer, that this computer had not read when it recorded | The record that anchor names. The local version is kept as history and is not reflected, as a newer remote revision is never overwritten (`project/instrukt/spec/feature/work/github-record`, Writer and convergence). |
| No version of its own | The record GitHub's newest anchor on the commit names, as the runner read it. |

### Verdict publication

Evidence is recorded before anything of it is published, and recording does
not wait for GitHub. Each tick publishes what is outstanding for an opted-in
project, whether or not that project's todos module is on.

| Publication | Published once | By |
| --- | --- | --- |
| The evidence record on the commit, then its anchor | GitHub has the commit in the project's repository. Evidence of a commit GitHub does not have yet waits, and that is no failure. | The computer that recorded the evidence, whether or not it observes the project's pull requests. |
| The review status on the commit, for a head verdict or a derived approval | The anchor is confirmed. | That computer, and any computer that reads the anchored record and finds the status missing. |
| A pull-request review on each eligible pull request whose current head is the commit | The anchor is confirmed and the runner observed that pull request at that head. | The computer that observes the project's pull requests. |

What a pull request receives follows the kind of verdict its head has.

| Verdict of the head | Pull-request review |
| --- | --- |
| Head verdict | Approving, or change-requesting, and carrying the report with its findings. |
| Derived approval | Approving, carrying the coverage: every covering item with the reference to its evidence. No review of the candidate commit took place, and none is started for the pull request. |

A covering item is referenced by the evidence its producer keeps, as
workflow's finding names it.

| Covering item | Reference |
| --- | --- |
| A head approved by a head verdict | That head's evidence record on its commit. |
| A todo's head approved in the todo's record | The todo, the approval bound to that exact commit with its report's digest, and the todo's pull request where its review is reflected (`project/instrukt/spec/feature/work/github-record`, Artifacts and stamps). No evidence record exists for such a head and none is written. |
| An exempted commit | Its evidence record. |
| A reproduced merge, documentation fold or revert | The commits it was reproduced from, named as reproduced. |

Every eligible pull request at a commit thus receives its own native review
from the commit's one verdict: the pull request a request was addressed to and
every other alike.

A pull request's review is owed for as long as the runner observes that pull
request eligible at that head and GitHub carries no review of that commit from
the reviewer bot on it. One that becomes eligible at that head later is owed
it then, and so is one that was closed or made a draft before its review was
written and is eligible at the same head again.

Only a commit's current evidence is published; a version replaced before it was
published is not. Before writing, publication recognises what GitHub already
carries from the reviewer bot: its record and anchor on the commit, its status
of that name on the commit, and its review of that commit on that pull request.
A write that already landed is not repeated. Whatever is missing is completed
from the commit's current evidence, on whichever computer serves the project;
nothing is reviewed again to complete a publication.

An unavailable GitHub leaves a publication outstanding with its cause, and the
next tick retries it. A destination GitHub refuses, such as a pull request
closed before its review was written, is recorded as refused with its cause and
is not retried while what was observed of it holds; the refusal ends when the
destination is owed again.

### Review evidence storage

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review.db` | Head verdict versions with their reports, and exemptions, as recorded on this computer. Authoritative until reflected. | `instrukt review record` and `instrukt review exempt` only: commands their caller waits on. | `state/review.lock` |
| `state/review-publication.db` | Derived approvals with their coverage; the evidence records GitHub carries for the commits this computer answers for, as read; and per piece of evidence and destination whether GitHub carries it, with the cause of an outstanding or refused one. | The tick only. | `state/review-publication.lock` |

No background work writes the evidence store and no command a caller waits on
writes the publication store, so recording a verdict never waits behind a tick
(`sdlc/policy/state-storage`). Neither store takes the state database's writer
lock, and readers take no lock. The publication store refers to evidence by
commit and version, and everything in it is read again from GitHub or derived
again when it is missing, so the two stores share no transaction. The pages the
runner reads evidence records from are the runner's own store
(`project/instrukt/spec/feature/runner/pull-request-review`, Observation
store).

Evidence is current for as long as it can still admit something: a commit's
current verdict, and an exemption, while the canonical branch does not contain
the commit and a branch of the project still does; evidence not yet reflected
and an outstanding publication until it is published or refused; a derived
approval while its candidate is the integration head. Everything else is
history: replaced versions, evidence of a commit the canonical branch contains
or no branch reaches any more, and completed publications. History is retired
after a retention period. Current evidence and outstanding publications are
never retired by age. The evidence store retires its history inside a recording
command, the publication store inside a tick.

### Evidence when the serving computer changes

Both stores belong to one computer; the evidence belongs to the project and
moves with it through its evidence records.

| Duty | Belongs to | Lasts |
| --- | --- | --- |
| Reflecting accepted evidence as its record and anchor | The computer that accepted it | Until GitHub carries it, for as long as the project is opted in. It does not end when that computer stops observing the project's pull requests. |
| Answering for a project's commits | The computer the project is admitted on | While it is admitted there. |

A computer serves a project's automatic review while the project is admitted
there, and the orderly end of that service is the configuration write that ends
the admission. Work answers which current evidence this computer holds for the
project that GitHub does not carry yet. Configuration refuses that write while
there is any and names its door, and installation inspection reports the same
evidence
(`project/instrukt/spec/feature/configuration/layered-configuration`, Automatic
review enablement). A computer that left this way has reflected the latest
version of everything it accepted, with its report, so the record GitHub's
newest anchor names is that computer's latest decision and nothing older is
current anywhere.

A computer that starts serving a project reads the anchored records of the
commits it has to answer for — eligible heads, the heads a candidate merges
and the candidate's own commits — and from them recognises verdicts, counts
exemptions and derived approvals, and completes every publication still
missing, with the recorded report and without another review.

### Readiness for automatic review

An opted-in project needs two scoped installation credentials. No other
credential changes.

| Credential | Identity | Permissions | Used for |
| --- | --- | --- | --- |
| Review observation | The project identity | Read access to pull requests, checks and commit statuses | The tick's pull-request listing and its reads of evidence records and their anchors. |
| Review publication | The reviewer identity | Write access to pull requests, commit statuses and checks, read access to contents | Evidence records and their anchors, the review status and pull-request reviews, with the reads that recognise what GitHub already carries. |

Work verifies both by requesting each credential with exactly those
permissions, where the project's opt-in is admitted and in
`instrukt install --check`
(`project/instrukt/spec/feature/configuration/layered-configuration`,
`project/instrukt/spec/feature/runner/module-installation`). A missing
installation or permission is an unmet finding naming the App, the repository,
the missing permission and the configuration repair door. Every publication is
thereby a permission of the reviewer identity that the credential request
establishes; none rests on a repository setting readiness cannot read.
Verification writes no state. The credentials that observe CI and that reflect
build reviews are unchanged: a project that has not opted in needs none of
these permissions.
<!-- /planned:event-driven-pr-review-gate -->

## Use cases

### UC-GA1: See bot authorship and receive an independent native review

```gherkin
Given a developer has configured distinct project, author and reviewer Apps
When an author role opens its implementation pull request and a reviewer role approves it
Then GitHub shows the author bot and the different reviewer bot
And shared record, issue and board maintenance shows the project bot
```

### UC-GA2: Recover reflected work written by the configured bots

```gherkin
Given a developer's reflected work was written by a configured verified App bot
When they rebuild their local record from GitHub
Then the bot's NONE association does not exclude its valid record revisions
And schema, artifact and evidence checks still apply
But a similarly named unconfigured actor gains no record trust
```

<!-- planned:event-driven-pr-review-gate -->
### UC-GA3: See an automatic review's verdict where GitHub can enforce it

```gherkin
Given a developer's pull request head has been reviewed automatically
When the reviewer's verdict is published
Then the pull request shows the reviewer bot's approving or change-requesting review with its findings
And the reviewed commit carries the reviewer bot's matching success or failure review status
But a later head of that pull request carries no status from that verdict
```
<!-- /planned:event-driven-pr-review-gate -->

## Known caveats

- GitHub App installation/permission availability and provider artifact/board
  capabilities still bound remote work; token issuance supplies no missing API.
- Human approval attribution remains the approvals contract. App configuration
  does not grant a bot permission to speak for an authorized human.

<!-- planned:event-driven-pr-review-gate -->
- Review evidence recorded on a computer and not yet reflected exists only
  there. A computer that stops ticking for the project without the
  configuration write that ends its admission — it is switched off, or loses
  the project's checkout or its home — keeps that evidence in its store and
  reflects it when it ticks for the project again. Until then no other computer
  sees it: an owed head is reviewed again, an exemption has to be recorded
  again, nothing is admitted on its account, and an approval whose later
  needs-work version was never reflected stays the newest GitHub shows.
- A head verdict's report is published on the reviewed commit as its evidence
  record and is readable by everyone who can read the repository, also for a
  head that has no pull request.
- An evidence record and its anchor are GitHub's check and status data. GitHub
  removes that data after the retention its owner configures, and a record
  whose content no longer matches its anchor is not read. A commit whose
  current record cannot be read whole has no recognised verdict in instrukt and
  is asked for again where it is owed. GitHub's branch protection reads the
  review status alone.
- A review whose reviewing session can no longer record it leaves no evidence.
  Its head is covered by a review of the commit or of the candidate that
  contains it.
- A revision is refused in a turn that a review request for the same commit
  also entered. The reviewing session records it in a turn of its own.
- Work verifies a reviewer's independence only against the sessions the
  reviewed commits name. For a commit that names none it verifies nothing about
  its author.
<!-- /planned:event-driven-pr-review-gate -->

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [project/instrukt/spec/feature/configuration/configuration-skill](../configuration/configuration-skill.md)
- [project/instrukt/spec/feature/configuration/owned-files](../configuration/owned-files.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md)
- [project/instrukt/spec/feature/repository/attributed-commit](../repository/attributed-commit.md)
- [general/concept/lineage-vs-authorship](../../../../general/concept/lineage-vs-authorship.md)
- https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app
- https://github.com/actions/create-github-app-token — GitHub's reference token issuance and bot commit-author example.
