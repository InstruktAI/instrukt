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
one status.

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

The first verdict admitted for a commit settles its review. A later verdict for
the same commit is a new version, admitted only from the session that recorded
the current one, as when a reviewer judges its own findings again against the
author's evidence. The latest version is current and earlier ones are kept as
history.

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
| Range | The pinned base is the request's for a requested review. For an unrequested one it is the merge base of the head with the project's canonical branch as the repository holds it, so the review covers the whole head, as `sdlc/procedure/lifecycle/work/review-commits` pins its candidate. `--base` states where the review started and is admitted when it is the pinned base or one of that base's ancestors. The caller cannot narrow the range. | The pinned base. |
| Independence | No commit after the pinned base up to the head names the reviewing session in its `Session:` trailer. | The commits that name the caller. The door is a review by a session none of them names. |
| Settlement | The commit has no verdict yet, or the caller is the session that recorded its current version. | The settled verdict and `instrukt review show`. |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

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

| Evidence | Evidence record |
| --- | --- |
| Head verdict version | A record comment on the reviewed commit carrying the version whole: verdict, pinned base, complete report, reviewing session, request and the commits that name no authoring session. |
| Derived approval | A record comment on the candidate commit carrying the canonical head it was derived against and every covering item. |
| Exemption | A record comment on the exempted commit carrying the basis and who declared it. |

The reviewer identity writes every evidence record, and a record counts only
from that identity, recognised by its verified bot identity; a comment of the
same shape from anyone else is not evidence. A record declares its record
schema version and is read under the compatibility rule of
`project/instrukt/spec/feature/work/github-record` (Record schema
compatibility). One that does not fit a single comment is carried in numbered
parts and counts only when every part is present. A record is never rewritten:
a later version is a later record.

The evidence record is the first thing published of any evidence. The review
status and the pull-request reviews are written only once the record is
confirmed on GitHub, so whatever GitHub shows of a verdict has its complete
evidence beside it. The review status alone is never evidence in instrukt: a
commit that carries the status and no record has no recognised verdict.

Which evidence is current for a commit is resolved in this order, and every
reader of review evidence asks work for this answer:

| Work holds for the commit | Current evidence |
| --- | --- |
| A version recorded on this computer that is not yet reflected, or that is the last record GitHub carries | That version. What GitHub still shows of an earlier version decides nothing: an approving status that a later needs-work version replaced approves nothing. |
| A version recorded on this computer that GitHub moved past before it was reflected: GitHub carries a later record, written by another computer, that this computer had not read when it recorded | The record GitHub carries. The local version is kept as history and is not reflected, as a newer remote revision is never overwritten (`project/instrukt/spec/feature/work/github-record`, Writer and convergence). |
| No version of its own | The last record GitHub carries on the commit, as the runner read it. |

### Verdict publication

Evidence is recorded before anything of it is published, and recording does
not wait for GitHub. Each tick publishes what is outstanding for an opted-in
project, whether or not that project's todos module is on.

| Publication | Published once |
| --- | --- |
| The evidence record on the commit | GitHub has the commit in the project's repository. Evidence of a commit GitHub does not have yet waits, and that is no failure. |
| The review status on the commit, for a head verdict or a derived approval | The evidence record is confirmed. |
| A pull-request review on each eligible pull request whose current head is the commit | The evidence record is confirmed and the runner observed that pull request at that head. |

What a pull request receives follows the kind of verdict its head has.

| Verdict of the head | Pull-request review |
| --- | --- |
| Head verdict | Approving, or change-requesting, and carrying the report with its findings. |
| Derived approval | Approving, carrying the coverage: every covering item, and for each approved head a reference to that head's evidence record. No review of the candidate commit took place, and none is started for the pull request. |

Every eligible pull request at a commit thus receives its own native review
from the commit's one verdict: the pull request a request was addressed to and
every other alike, and one that becomes eligible at that head later receives it
then.

Only a commit's current evidence is published; a version replaced before it was
published is not. Before writing, publication recognises what GitHub already
carries from the reviewer bot: its evidence record on the commit, its status of
that name on the commit, and its review of that commit on that pull request. A
write that already landed is not repeated. Whatever is missing is completed
from the evidence record, on whichever computer serves the project; nothing is
reviewed again to complete a publication.

An unavailable GitHub leaves a publication outstanding with its cause, and the
next tick retries it. A destination GitHub refuses for good, such as a pull
request closed before its review was written, is recorded as refused with its
cause and is not retried.

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

Both stores belong to one computer; the evidence belongs to the project and
moves with it through its evidence records. A computer that starts serving a
project reads the records of the commits it has to answer for — eligible heads,
the heads a candidate merges and the candidate's own commits — and from them
recognises verdicts, counts exemptions and derived approvals, and completes
every publication still missing, with the recorded report and without another
review.

### Readiness for automatic review

An opted-in project needs two scoped installation credentials. No other
credential changes.

| Credential | Identity | Permissions | Used for |
| --- | --- | --- | --- |
| Review observation | The project identity | Read access to pull requests | The tick's pull-request listing and its reads of evidence records. |
| Review publication | The reviewer identity | Write access to pull requests and to commit statuses, read access to contents | Evidence records, the review status and pull-request reviews, with the reads that recognise what GitHub already carries. |

Work verifies both by requesting each credential with exactly those
permissions, where the project's opt-in is admitted and in
`instrukt install --check`
(`project/instrukt/spec/feature/configuration/layered-configuration`,
`project/instrukt/spec/feature/runner/module-installation`). A missing
installation or permission is an unmet finding naming the App, the repository,
the missing permission and the configuration repair door. Verification writes
no state. The credentials that observe CI and that reflect build reviews are
unchanged: a project that has not opted in needs neither permission.
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
  there. If that computer stops serving the project before a tick reflects it,
  the evidence does not move: an owed head is reviewed again, an exemption has
  to be recorded again, and nothing is admitted on its account.
- A head verdict's report is published on the reviewed commit as its evidence
  record and is readable by everyone who can read the repository, also for a
  head that has no pull request.
- An evidence record is a comment, and an account that can delete comments on
  the repository can remove one. A commit whose record is gone has no
  recognised verdict in instrukt and is asked for again where it is owed.
  GitHub's branch protection reads the review status alone.
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
