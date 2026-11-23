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

The status is named `instrukt/review`. It names the exact commit that was
reviewed; a later head never inherits it. The reviewer bot is the status's only
publisher; a status of the same name from another identity is not a verdict.
The reviewer identity is the App the `commit-reviewer` role selects.

The verdict is a judgment of the commit; the pull-request review is its
delivery to one pull request. Every eligible pull request whose current head is
that commit receives its own review of the one verdict, and the commit receives
one status. The status belongs to the commit and admits it wherever it is
proposed, so a verdict is a judgment of the whole range its pinned base fixes
(Admission of review evidence, Range). The status and the reviews are all that
instrukt publishes of a review; the evidence stays in work's stores (Review
evidence storage).

### Review evidence

Work keeps three kinds of review evidence for exact commits that no todo's
record carries. A head verdict and a derived approval are a commit's verdict;
an exemption is not.

| Evidence | Records | Recorded by |
| --- | --- | --- |
| Head verdict | An independent review's outcome for one commit: the commit, the pinned base the review started from and whether that base lies on the canonical branch, the verdict, the complete report, the reviewing session, the review request it answers when it answers one, and which of the reviewed commits name no authoring session. | The reviewing session, with `instrukt review record`. |
| Exemption | The decision that a commit owes no independent review under `general/policy/commits`, with its stated basis, the session that declared it when one resolves, and whether the commit names an authoring session. | The commit's author, with `instrukt review exempt`. |
| Derived approval | Workflow's finding that an integration candidate's content is covered (`project/instrukt/spec/feature/workflow/integration-gate`), with the canonical head it was derived against and every covering item. | The runner's tick, through work's command contract. No agent command records one. |

| Command | Contract |
| --- | --- |
| `instrukt review record --head <sha> --base <sha> --verdict approve\|needs_work --report <file\|->` | Records the calling session's head verdict of the commits after the base up to the head. |
| `instrukt review record --head <sha> --base <sha> --verdict approve --report <file\|-> --reviewer <session>` | Records the approving head verdict that the named session reached and handed over (A handed-over approval). |
| `instrukt review exempt --commit <sha> --basis <text>` | Records an exemption for a commit the canonical branch does not contain yet. |
| `instrukt review show <sha>` | What this computer holds for the commit: its verdict versions, its exemption and its derived approval, each with its pinned base and where it was recorded, every publication with its state and the cause of an outstanding or refused one, and a verdict recognised as already published where this computer holds none. |
| `instrukt review export --to <file>` | Hands the project's review evidence held on this computer off into one file (Evidence when the serving computer changes). |
| `instrukt review import <file>` | Takes an export's evidence in (Evidence when the serving computer changes). |

The first verdict admitted for a commit settles its review. A later version
exists only as a revision: the session that recorded the current version judges
its own findings again, as when the author disputes them with evidence and no
new commit. The latest version is current and earlier ones are kept as history.

A verdict and an exemption are separate facts of a commit. Recording, importing
or publishing one never replaces, hides or retires the other, and work's answer
on a commit reports both.

A head verdict is its reviewer's. Whether the review was requested, when it
took place and whether the project has opted in decide nothing about its
admission. Recording needs no opt-in; publication does (Verdict publication).

### Admission of review evidence

`instrukt review record` admits a verdict only when all of these hold. They are
facts of the reviewed commit and of the reviewing session's recorded relation
to it, and they bind every caller, a session without a role included
(`project/instrukt/spec/feature/configuration/roles-and-capabilities`).

| Invariant | Rule | The refusal names |
| --- | --- | --- |
| Repository | `--head` is the full id of a commit present in the repository of the project the caller stands in. | The commit and the project. |
| Reviewer | The reviewing session is the caller, a session the selected runtime adapter recognises, resolved as for an attributed commit (`project/instrukt/spec/feature/repository/attributed-commit`, Resolution). With `--reviewer` it is the named session (A handed-over approval). | That no session resolves. |
| Request | Read from the runner (`project/instrukt/spec/feature/runner/pull-request-review`, One review per commit). While a request for exactly this project and commit is queued, selected or handed over, the review is that request's: only the session it was handed over to records it, and every other caller is refused, also while no session holds it yet. A commit with no such request is unrequested, and its verdict is a one-off review's whoever reviewed, a session whose request has ended included. | The request with its standing, and the session that holds it once it was handed over. |
| Range | The pinned base is fixed by instrukt and the caller cannot narrow it. For a head that shares history with the project's canonical branch it is their merge base, so the verdict covers everything the head adds to that branch, whatever branch a pull request at that head targets. For a head that shares none it is the merge base of the head with its pull request's base branch, as a request of that commit carries it, and the verdict is recorded as not covering the canonical branch; such a head is refused while no request of it is on record. A request of the commit, live or ended, carries the pinned base; work computes it from the repository for a commit that has had none, as `sdlc/procedure/lifecycle/work/review-commits` pins its candidate. A later version of a settled commit keeps the pinned base of the version it replaces. `--base` states where the review started and is admitted when it is the pinned base or one of that base's ancestors. | The pinned base. |
| Independence | No commit after the pinned base up to the head names the reviewing session in its `Session:` trailer. | The commits that name the reviewing session. The door is a review by a session none of them names. |
| Settlement | The commit has no verdict yet, or the verdict is a revision: the caller is the session that recorded the current version, and the turn it has in progress was not opened by a review request for this commit. | The settled verdict and `instrukt review show`; for the session that recorded it, which of the answers below applied. |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

A review request is answered in the turn it opened. A verdict the settling
session offers in such a turn, for a commit that is already settled, answers
that request a second time. Which input opened the session's turn in progress
is the runner's answer
(`project/instrukt/spec/feature/runner/pull-request-review`, One review per
commit). Work asks it only for a settled commit and its settling session, so
every other verdict is admitted or refused from the stores alone.

| The runner answers | The verdict |
| --- | --- |
| The turn was opened by something other than a review request for this commit | Is admitted as a revision. |
| The turn was opened by a review request for this commit | Is refused; no version is recorded. |
| The session has no turn in progress | Is refused; a revision is recorded within a turn of its session. |
| The runtime cannot say | Is asked for again until the command's own wait has passed, then refused as not decidable. Nothing is recorded and the refusal names the runtime as its cause. The session keeps its report and records again in the same turn; an offer made in a later turn is judged by that turn. |

A commit without a `Session:` trailer names no session, and that is its true
state (`general/concept/lineage-vs-authorship`). Work refuses nothing on that
and reads nothing into it: the verdict records such a commit as naming no
authoring session, and counts like any other verdict. Independence from
whoever wrote such a commit is the reviewing procedure's own duty and is not a
fact work established.

The caller names no pull request and no request. Where a verdict is delivered
follows from which eligible pull requests have the commit as their current
head, as the runner observed them. A verdict stays recordable for its exact
commit after the pull request has moved on; it is that commit's and no
other's.

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

### A handed-over approval

An approval its reviewing session returned through native messaging and did
not record is admitted from that session's own conversation.
`instrukt review record --reviewer <session>` names the session; the caller
supplies the head, the base and the report, and adds no judgment of its own.

| Checked | Rule |
| --- | --- |
| The session's own words | The named session's conversation, read through the runner (`project/instrukt/spec/feature/runner/history`), holds a message which that session itself authored and which names the full head commit, the full base commit and the SHA-256 digest of the supplied report. |
| The verdict | The supplied report ends in the approving verdict line, in the form `sdlc/procedure/lifecycle/review` fixes for a report. A needs-work result is not admitted this way. |
| The commit | Repository, Request, Range, Independence and Report hold as above with the named session as the reviewing session, and the commit has no verdict. |

The verdict is recorded as the named session's, with the caller as the session
that entered it, and counts like any other head verdict. A conversation that
cannot be read, a message that names another commit or digest, a report
without that verdict line, and a head other than the one the message names
each leave the approval unrecorded, and the refusal names which. Such a head
owes its review like any other.

### Verdict publication

Evidence is recorded before anything of it is published, and recording does
not wait for GitHub. Publication is the tick's, on a computer the project is
admitted on for automatic review
(`project/instrukt/spec/feature/configuration/layered-configuration`, Automatic
review enablement), whether or not that project's todos module is on. It
publishes the current verdict of each commit this computer holds evidence for:
the current version of its head verdict, or its derived approval. An exemption
publishes nothing, and evidence this computer handed off is not published from
it.

A version is named by the digest of its report, and a derived approval by the
digest of its coverage. The status carries that name in its description and
the review in its body.

| Destination | Owed while |
| --- | --- |
| The review status on the commit | GitHub has the commit in the project's repository, and the reviewer bot's newest status named `instrukt/review` on it does not have the current verdict's state together with its name. |
| A pull-request review on each eligible pull request whose current head is the commit | The runner observes that pull request eligible at that head, and the pull request has no review by the reviewer bot of exactly that commit with the current verdict's native state and its name in the body. |

Each destination follows the current verdict. A revision is published over an
earlier version of the same commit, also when it reverses it; a status or a
review GitHub no longer shows is written again from the evidence; and a
destination that already shows the current verdict is not written again, so a
publication repeated after an interruption leaves no duplicate. What GitHub
shows is recognised by the reviewer bot's verified identity. Nothing is
reviewed again to complete a publication.

Which commits are published follows from the evidence held, not from what the
runner observes: a commit that heads no pull request any more still receives
its status. Evidence of a commit GitHub does not have yet waits, and that is
no failure.

What a pull request receives follows the kind of verdict its head has.

| Verdict of the head | Pull-request review |
| --- | --- |
| Head verdict | Approving, or change-requesting, and carrying the report with its findings. A report longer than a review body holds is carried up to that length and marked as shortened; `instrukt review show` gives it whole. |
| Derived approval | Approving, carrying the coverage: every covering item with the reference to its evidence. No review of the candidate commit took place, and none is started for the pull request. |

A covering item is referenced by the evidence its producer keeps, as
workflow's finding names it.

| Covering item | Reference |
| --- | --- |
| A head approved by a head verdict | The commit, the reviewing session and the digest of the approving version's report. |
| A finalized commit approved in its todo's record | The todo, the build-review approval bound to that exact commit and its report's digest, and the todo's pull request when that approval was reflected there as a review (`project/instrukt/spec/feature/work/github-record`, Artifacts and stamps). |
| A reviewed preparation head approved in its todo's record | The todo, that approval bound to the exact commit and its report's digest (`project/instrukt/spec/feature/work/stamps`, Historical preparation-head stamps). No pull-request review exists for it and none is cited. |
| An exempted commit | The commit, the exemption's basis and who declared it. |
| A reproduced merge, documentation fold or revert | The commits it was reproduced from, named as reproduced. |

Every eligible pull request at a commit thus receives its own native review
from the commit's one verdict: the pull request a request was addressed to and
every other alike. One that becomes eligible at that head later is owed it
then, and so is one that was closed or made a draft before its review was
written and is eligible at the same head again.

An unavailable GitHub leaves a publication outstanding with its cause, and the
next tick retries it. A destination GitHub refuses, such as a pull request
closed before its review was written, is recorded as refused with its cause and
is not retried while what was observed of it holds; the refusal ends when the
destination is owed again.

A computer that holds no evidence for an eligible head recognises a verdict the
reviewer bot already published there: the reviewer bot's newest
`instrukt/review` status on the commit and a review by the reviewer bot of
that commit on the pull request carry the same name and agree in state. Such a
head carries the reviewer identity's verdict and is asked for no review.
Recognition records what GitHub shows and nothing more: it supplies no report
and no evidence, earns no candidate coverage, and no further publication is
written from it. A status without such a review, and a review without such a
status, recognise nothing.

### Review evidence storage

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review.db` | Head verdict versions with their reports, and exemptions, as recorded on or imported to this computer; imported derived approvals; and which head verdicts an export handed off. | `instrukt review record`, `exempt`, `export` and `import` only: commands their caller waits on. | `state/review.lock` |
| `state/review-publication.db` | The derived approvals the tick recorded, with their coverage; per current verdict and destination what GitHub shows of it, with the cause of an outstanding or refused one; and the verdicts recognised as already published. | The tick only. | `state/review-publication.lock` |

No background work writes the evidence store and no command a caller waits on
writes the publication store, so no command waits behind a tick
(`sdlc/policy/state-storage`). Neither store takes the state database's writer
lock, and readers take no lock. The publication store refers to evidence by
commit and version and shares no transaction with the evidence store. What it
holds of GitHub is read again when it is missing, and the derived approval of
the integration head is derived again. The derived approval of a candidate the
canonical branch has since taken cannot be derived again and is kept there
until it is retired.

Evidence is current for as long as it can still admit something: a commit's
current verdict, and an exemption, while the canonical branch does not contain
the commit and a branch of the project still does; an outstanding publication
until it is published or refused; a derived approval while its candidate is
the integration head. Everything else is history: replaced versions, head
verdicts an export handed off, evidence of a commit the canonical branch
contains or no branch reaches any more, and completed publications. History is
retired after a retention period. Current evidence and outstanding publications
are never retired by age. The evidence store retires its history inside a
recording command, the publication store inside a tick.

### Evidence when the serving computer changes

Both stores belong to one computer. GitHub carries the two publications and no
evidence, so review evidence reaches another computer through an export and an
import of the evidence store and through nothing else.

| Command | Contract |
| --- | --- |
| `instrukt review export --to <file>` | Writes one cut of the project's review evidence on this computer: every current head verdict with all its versions and reports, every current exemption and every derived approval it keeps, with what is not yet published and what concerns commits GitHub does not have. It holds `state/review.lock`, so a recording command falls wholly before the cut or wholly after it. The file is complete on disk before the cut is committed, and with the cut the exported head verdicts are handed off. |
| `instrukt review import <file>` | Takes an export in, holding `state/review.lock`, commit by commit, and removes or replaces nothing the importing computer holds. |

A handed-off head verdict is history on the exporting computer. That computer
publishes nothing further of it and admits no further version for its commit;
the refusal names the export and its door, which is recording on the computer
that imported it or importing an export of it here. Evidence recorded after
the cut is not part of it. A later export carries it, and carries again what
earlier exports handed off for as long as that is kept, so a lost file loses
no evidence.

| The export holds for a commit | The importing computer holds | Result |
| --- | --- | --- |
| Verdict versions | None | The export's versions in their order; the latest is current. |
| Versions that continue the ones held | The first of them | The further versions are appended. |
| Versions | The same, or those and later ones | Nothing changes. |
| Versions the importing computer lacks | Versions of its own the export lacks | Those held stay current. The export's are kept as history and the import names the commit. |
| An exemption or a derived approval | Any | It is added where none is held for that commit; where one is held, nothing changes. A verdict and an exemption of one commit never replace each other. |

Imported evidence is the importing computer's from then on. It keeps its
reviewing session and where and when it was recorded, counts like evidence
recorded there, and has every destination of its current verdicts outstanding
until GitHub is found to show them or they are written. Evidence this computer
handed off and imports again is current here again. The same file imported
twice changes nothing. A file whose content does not match the digest it
declares is refused whole. Evidence of a commit the importing repository does
not hold is kept and is published once GitHub has the commit.

Neither command needs the project to be opted in, waits for GitHub or takes
the publication lock: an export reads the derived approvals as every reader of
the publication store does, and an import keeps them in the evidence store.

A computer that starts serving a project without an import holds no evidence.
It recognises what the reviewer bot already published on eligible heads
(Verdict publication) and asks for every other owed head.

### Readiness for automatic review

An opted-in project needs two scoped installation credentials. No other
credential changes.

| Credential | Identity | Permissions | Used for |
| --- | --- | --- | --- |
| Review observation | The project identity | Read access to pull requests | The tick's pull-request listing. |
| Review publication | The reviewer identity | Write access to pull requests and to commit statuses | The review status and pull-request reviews, with the reads that recognise what GitHub already shows. |

Work verifies each by requesting the credential with exactly those
permissions. A missing installation or permission is an unmet finding naming
the App, the repository, the missing permission and the configuration repair
door. Verification writes no state. Where readiness is consulted belongs to
`project/instrukt/spec/feature/configuration/layered-configuration` (Automatic
review enablement). The credentials that observe CI and that reflect build
reviews are unchanged: a project that has not opted in needs none of these
permissions.
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
- Review evidence exists only on the computer that recorded or imported it. A
  computer that is lost, or loses its home, without an export takes with it
  what it had not published: the computer that serves next has an owed head
  reviewed again, an exemption has to be recorded again, and an approval
  GitHub already shows stays the newest it shows although a later needs-work
  version existed.
- Evidence recorded on a computer the project is not admitted on is published
  by no one until that computer is admitted or the evidence is exported and
  imported where the project is served.
- A head verdict's report is published on every pull request its commit heads
  and is readable by everyone who can read the repository.
- A review status GitHub no longer shows is written again by the computer that
  holds the commit's evidence. A computer that holds none cannot restore it.
- A verdict recognised as already published supplies no report. A further pull
  request at that head receives no review of its own from a computer that
  holds no evidence for the commit, and the head earns no candidate coverage
  there.
- A handed-over approval is admitted only from a conversation instrukt can
  still read and a report that ends in the fixed verdict line. Without them
  its head owes a review.
- A revision is refused in a turn that a review request for the same commit
  opened; the reviewing session records it in another turn. The answer rests
  on the input that opened the turn, so a request that a runtime steers into a
  running turn is not seen by it
  (`project/instrukt/spec/feature/runner/pull-request-review`, One review per
  commit).
- A verdict of a head that shares no history with the canonical branch covers
  what the head adds to its pull request's base branch and nothing on the
  canonical branch.
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
