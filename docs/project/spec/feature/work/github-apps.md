---
description: Role-selected GitHub App bot identities, protected keys and installation tokens for visible bot authorship and independent native reviews.
binding_reservations:
  UC-GA2: github-human-agent-reflection
---
# GitHub Apps — Spec

## What it is

instrukt writes to GitHub as configured GitHub Apps. The bot is visible on
issues, their artifacts, bugs, board updates, pull requests and reviews.
Authoring and review use separate bots, so GitHub's native prohibition on
author self-approval does not prevent independent review. Account lineage is
distinct from who did the work (`general/concept/lineage-vs-authorship`).

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
`~/.instrukt/agents.yaml`. Work recorded under a retired split orchestrator role
(`prepare-orchestrator`, `work-orchestrator`) selects the current mapping of
`todo-orchestrator`, the role agents migration replaces the split entries with.
An omitted mapping uses `github.project_app`, including an unknown role resolved
through `default`. No per-task model override,
command argument or profile order selects the GitHub actor. Background issue,
artifact, bug and board maintenance uses the project identity. Role-driven pull
request authoring and reviews use the selected role identity. Human work
performed directly on GitHub keeps its human identity; instrukt's issue and
board maintenance still uses the project App. This contract changes no human
login/account association or runtime permission mode.


| Profile reference | Shipped identity | Responsibility |
| --- | --- | --- |
| `project` | `instruktai[bot]` | Issues, their artifacts, bugs and the board; roles without a specific mapping. |
| `author` | `instruktai-author[bot]` | Authoring roles, their pull requests and bot-authored commits. |
| `reviewer` | `instruktai-reviewer[bot]` | Reviewer roles and their native GitHub reviews. |

The shipped role compositions live in the shipped `agents.yaml` defaults under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. An
independent reviewer uses a different App/bot from the pull request author;
using a different session of the same App does not satisfy GitHub's native
restriction. Review stamps retain their own independence and exact-evidence
requirements; different bots alone do not prove review occurred.

### Configuration skill and installation

The configuration skill reuses the three Apps on an
installation that already has them, verifying their configured identity and
repository installation instead of creating duplicates. On other installations
it guides creation and installation of the three Apps and records their actual
public identities; globally unique slugs can differ from the shipped names.
It writes private keys only through `instrukt secrets set`, into
`~/.instrukt/secrets.env` under the owned-files contract. No key or token is
carried in a configuration report, repo file or assignment brief.

Setup verifies the App installation reaches the project's repository and has
the permissions required by the operation: issues/artifacts and board
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
cause. During one todo's reflection, a failure of the App its pull request,
review or board update selects is local to that todo when that App's credential
cannot be issued because of a setup or identity mismatch or GitHub refuses that
App's request, or when the reviewer App's verified bot is the pull request's
author. A failed issuance of the project credential every reflection request
shares, an issuance GitHub throttles, may be throttling or cannot serve, a rate
limit, an unreachable GitHub and GitHub refusing an issued credential are
command-wide. What a failure stops depends on the operation it interrupts:

- Issue, artifact, pull request and review reflection: a command-wide failure
  stops the command's remaining GitHub requests; a local failure keeps the
  todo's remaining reflection in that pass pending with the cause, and the pass
  continues with the next todo.
- Board setup at the start of reflection: when it cannot obtain the board
  account's credential or the board, board updates stay pending with the cause;
  a rate limit stops the command's remaining GitHub requests, and any other
  cause leaves the rest of reflection running.
- A todo's board update: its failure is kept for that update's own retry; a
  rate limit or an unreachable GitHub stops the command's remaining GitHub
  requests, and any other failure, an App setup mismatch or refusal and a
  refused credential included, lets the pass continue.

Rate limits retain their retry boundary: GitHub's `Retry-After` delay, counted
from when the throttled answer is received, or without one its reset, and the
later of the two when no requests remain. A secondary limit GitHub reports in the
errors of a successful GraphQL answer without a `Retry-After` waits a minute from
receipt instead of the reset, unless no requests remain. An HTTP 403 or 429
refusal is judged by its headers alone, never by its body: a 429, or a 403 with no
requests remaining or a `Retry-After`, is a rate limit with the boundary its
headers name, and any other 403 stays a refusal. Locally accepted
work and its pending reflection survive; no credential failure is reported as
remote success or as proof that an ambiguous write had no effect. Reflection's
existing retry-convergence rules remain in force
(`project/instrukt/spec/feature/work/github-record`).


### Read-only reflection readiness

Work verifies the project App, protected key, repository installation,
issue and account-board permission scopes, expiry and stable bot identities.
It reads repository issues, existing issue artifacts when an issue is available,
and the configured board or its account's project listing. Newer pages containing
only pull requests do not exclude older issues from artifact readiness checks.
Unavailable access or reflection capabilities are unmet findings with the
configuration repair door.
Read-only readiness creates no issues, artifacts, boards, fields or views.
It establishes declared write permissions and available read surfaces; it cannot
prove a future remote write, or artifact access on an empty repository, without
performing a mutation.

### Commit authorship

The App boundary supplies verified bot actor facts; commit-author resolution,
precedence and refusal belong to
`project/instrukt/spec/feature/repository/attributed-commit`.

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
| `instrukt review record --head <sha> --base <sha> --verdict approve --report <file\|-> --reviewer <session>` | Records the approving head verdict that the named session reached, entered by the calling session as that session's standing result (A handed-over approval). |
| `instrukt review exempt --commit <sha> --basis <text>` | Records an exemption for a commit the canonical branch does not contain yet. |
| `instrukt review show <sha>` | What this computer holds for the commit: its head verdict with its versions and with whether it stands here, was handed off by an export or was set aside by an import; its exemption and its derived approval, or a derived approval the tick withdrew with the commit found uncovered; each of these with its pinned base and where it was recorded; every publication with its state and the cause of an outstanding or refused one; and a verdict recognised as already published where this computer holds none. |
| `instrukt review export --to <file>` | Hands the project's review evidence that stands on this computer off into one file (Evidence when the serving computer changes). |
| `instrukt review import <file> [--standing imported\|held]` | Takes in an export (Evidence when the serving computer changes). |

The first verdict admitted for a commit settles its review. A later version
exists only as a revision: the session that recorded the current version
judges its own findings again, as when the author disputes them with evidence
and no new commit. The latest version is current and earlier ones are kept as
history.

<!-- planned:commit-rereview-reuses-reviewer -->
The session that recorded the current version of a commit's verdict that
stands here is that commit's reviewer of record. Work answers it with the
commit's verdict, together with the current version's report digest and
moment, which name and date the version (Verdict publication), and
`instrukt review show` names it. A verdict this computer handed off or set
aside has no reviewer of record here. The runner wakes a reviewer of record to
review later work (`project/instrukt/spec/feature/runner/requested-review`,
Reaching the reviewer). A wake records nothing of the commit's evidence and
changes nothing of who may record a version (Admission of review evidence,
Settlement): a verdict the woken session records for another commit is that
commit's own and admitted as that commit's.
<!-- /planned:commit-rereview-reuses-reviewer -->

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
| Reviewer | The reviewing session is the caller, a session the selected runtime adapter recognises, resolved as for an attributed commit (`project/instrukt/spec/feature/repository/attributed-commit`). With `--reviewer` it is the named session (A handed-over approval). | That no session resolves. |
| Request | Read from the runner (`project/instrukt/spec/feature/runner/pull-request-review`). While a request for exactly this project and commit is queued, selected or handed over, the review is that request's: only the session it was handed over to records it, and every other caller is refused, also while no session holds it yet. A commit with no such request is unrequested, and its verdict is a one-off review's whoever reviewed, a session whose request has ended included. | The request with its standing, and the session that holds it once it was handed over. |
| Range | The pinned base is fixed by instrukt and the caller cannot narrow it. A request of the commit, live or ended, carries the pinned base the runner selected for it (`project/instrukt/spec/feature/runner/pull-request-review`). For a commit that has had no request, work pins the head's merge base with the canonical branch, as `sdlc/procedure/lifecycle/work/review-commits` pins its candidate, and refuses a head that has none. A verdict whose pinned base does not lie on the canonical branch is recorded as not covering that branch. A later version of a settled commit keeps the pinned base of the version it replaces. `--base` states where the review started and is admitted when it is the pinned base or one of that base's ancestors. | The pinned base. |
| Independence | No commit after the pinned base up to the head names the reviewing session in its `Session:` trailer. | The commits that name the reviewing session. The door is a review by a session none of them names. |
| Settlement | This computer holds no verdict of the commit, or the verdict is a revision: the commit's verdict stands here, the caller is the session that recorded the current version, and, where a review request for this commit reached that session, the turn the session has in progress when it offers the verdict was not opened by such a request. | The settled verdict and `instrukt review show`; for the session that recorded it, which of the answers below applied; for a verdict this computer handed off, the export and its door (Evidence when the serving computer changes). |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

A review request is answered in the turn it opened. A verdict the settling
session offers in such a turn, for a commit that is already settled, answers
that request a second time. Only a session that a request of the commit
reached can be in such a turn, so work reads the commit's requests first
(`project/instrukt/spec/feature/runner/pull-request-review`).

| The commit's requests | The settling session's offer |
| --- | --- |
| None was handed over to that session, and none that delivery selected is still to be handed over | Is admitted as a revision from the stores alone. The reviewer of a commit that never had a request is always in this row. |
| One was handed over to that session, or one that delivery selected is still to be handed over | Is decided by which input opened the turn the session has in progress when it offers the verdict. |

Which input opened that turn is the runner's answer. Every other verdict is
admitted or refused from the stores alone.

| The runner answers | The offered verdict |
| --- | --- |
| Something other than a review request for this commit opened that turn | Is admitted as a revision. |
| A review request for this commit opened that turn | Is refused; no version is recorded. |
| The session has no turn in progress | Is refused; a revision is recorded within a turn of its session. |
| The runtime cannot say | Is asked for again until the command's own wait has passed, and is then refused. No version is recorded, and the refusal names the runtime that could not answer and its door: the same command with the same report, once the runtime answers. |

Every offered verdict is thus recorded or refused within the command's wait,
and a commit either has a current verdict or has none. A revision is the
commit's current version from the write that records it, and work answers the
commit by it before anything of it is published. Until then the earlier
version is current and stays published.

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
rests on. Until GitHub shows it as the commit's review status it stands only
while that finding does: workflow derives the candidate's coverage again on
each tick before anything of the approval is written
(`project/instrukt/spec/feature/workflow/integration-gate`).

| Workflow finds again | The derived approval GitHub does not show yet |
| --- | --- |
| Every commit covered | Is published, with the coverage of this finding. |
| A commit not covered | Is withdrawn. The tick records the withdrawal with the commit found uncovered, and the candidate carries no verdict. |
| Nothing, the repository being unable to answer | Stays recorded and unpublished in that tick. |

A derived approval that GitHub shows as the commit's status is withdrawn by
nothing.

### A handed-over approval

An approval its reviewing session returned through native messaging and did
not record is entered by another session.
`instrukt review record --reviewer <session>` names the reviewing session; the
entering session supplies the head, the base and the report, and adds no
judgment of the reviewed change.

The command is the entering session's statement that the report is the named
session's result for the head as it stands: it read that session's
conversation from the return of the report to its end and found no later
result for the head and no withdrawal. Whether a later message replaces,
withdraws or only mentions an approval is read by that session and is not
computed (`general/principle/intelligence-boundaries`). Work records the
statement with the entering session and establishes these facts beside it.

| Established by work | Rule |
| --- | --- |
| The return | The named session's conversation is read through the runner (`project/instrukt/spec/feature/runner/history`). A message that session itself authored names the full head commit together with the SHA-256 digest of the supplied report. Other digests in that message, and what other messages name, decide nothing. |
| The base | The base the entering session supplies is named, as a full commit, in such a message or in the supplied report. |
| The verdict | The supplied report ends in the approving verdict line, in the form `sdlc/procedure/lifecycle/review` fixes for a report. A needs-work result is not admitted this way. |
| The entering session | It resolves as the Reviewer row resolves a caller, is not the named session, and no commit after the pinned base up to the head names it in its `Session:` trailer. |
| The commit | Repository, Request, Range, Independence and Report hold as above with the named session as the reviewing session, and the commit has no verdict. |

A reviewing session returns the reviewed head with its report's digest and
keeps the base with the report
(`sdlc/procedure/lifecycle/work/review-commits`). What work establishes is
that this report is the named session's own, for this head and this range.
That it still stands is established by the statement and by nothing else: the
identifiers of a conversation admit no approval on their own, and no command
enters one without a session that states it.

The verdict is recorded as the named session's, with the entering session and
its statement, and counts like any other head verdict. A conversation that
cannot be read, no authored message that names the head together with this
report's digest, a base named in neither place, a report without that verdict
line, and an entering session that is the named session or that a reviewed
commit names each leave the approval unrecorded, and the refusal names which.
The named session records its own result without `--reviewer`. A head whose
approval is not entered owes its review like any other.

### Verdict publication

Evidence is recorded before anything of it is published, and recording does
not wait for GitHub. Publication is the tick's, on a computer the project is
admitted on for automatic review
(`project/instrukt/spec/feature/configuration/layered-configuration`), whether or not that project's todos module is on. It
publishes the current verdict of each commit this computer holds evidence for:
the current version of the head verdict that stands here, or its derived
approval. An exemption publishes nothing, a verdict this computer handed off
or set aside is not published from it, and a derived approval that GitHub
does not show yet is published only by a tick that found its coverage again
(Admission of review evidence).

A version is named by the digest of its report, and a derived approval by the
digest of its coverage. Each verdict also has its moment, which says when it
was decided and never when it was published. A head verdict is decided when
it is recorded, and decided again when an import keeps or makes it the
commit's verdict over another one. Its moment is later than the moment of the
verdict it was decided over.

| Decided by | The moment of the version |
| --- | --- |
| Its recording | The moment it was recorded. For a revision recorded at a moment that is not later than that of the version it replaces, the first moment after that one. |
| An import that keeps or makes its verdict the commit's and sets another one aside (Evidence when the serving computer changes) | Unchanged where it is already later than the moment of the current version of the verdict set aside. Otherwise the moment of the import, or the first moment after that other one where the import's is not later. |
| An import that brings the same version with a later moment | That later moment: the version was decided again where the file was written. |

The moments of a commit's head verdicts thus keep the order in which they
became current, across the versions of a verdict and across a verdict and one
an import set aside for it, whatever the clocks of the computers that held
them read. An import gives no version an earlier moment, moves the moment of
no other version and changes nothing else of one: when and where a version
was recorded stay with it, and `instrukt review show` gives both beside its
moment. A derived approval's moment is the moment the tick recorded the
coverage it is published with. The moment is kept with the verdict and
travels with it through an export and an import, and a publication carries
the moment its verdict has when it is written. The status carries the name
and the moment in its description, and the review carries both at the head of
its body, ahead of the report or the coverage.

| Destination | Owed while |
| --- | --- |
| The review status on the commit | GitHub has the commit in the project's repository, and the reviewer bot's newest status named `instrukt/review` on it does not have the current verdict's state together with its name and moment, or links no review although a pull request holds the reviewer bot's review of the current verdict. |
| A pull-request review on each eligible pull request whose current head is the commit | The runner observes that pull request eligible at that head, and the reviewer bot's latest review of exactly that commit on that pull request does not have the current verdict's native state together with its name and moment in the body. |

The status links the review. Its target address is the address GitHub gives
the reviewer bot's review that carries the current verdict, on the
lowest-numbered pull request that holds one. A tick writes a head verdict's
reviews before its status, so a commit that heads an eligible pull request
receives one status with the link. A status never waits for a review: while no
pull request holds one it is written without a link, and it is written once
more, with the link, when the first review of the current verdict exists. Of a
derived approval the status is written first, and a pull request receives its
review only once GitHub shows that status: nothing of a derived approval is on
GitHub before the status that admits its commit.

Each destination follows the current verdict. What a destination shows is the
reviewer bot's newest status on the commit and its latest review of the commit
on the pull request, never an earlier one. A revision is therefore published
over an earlier version of the same commit, also when it reverses it and when
it makes an earlier outcome current again: a verdict that returns after
another one was published is written as a new review. A verdict whose moment
an import moved is published again with that moment, over what GitHub shows
of it with the earlier one and over what it shows of the verdict set aside.
A status or a review
GitHub no longer shows is written again from the evidence; and a destination
whose latest already shows the current verdict is not written again, so a
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
| A finalized commit approved in its todo's record | The todo, the build-review approval bound to that exact commit and its report's digest, and the todo's pull request when that approval was reflected there as a review (`project/instrukt/spec/feature/work/github-record`). |
| A reviewed preparation head approved in its todo's record | The todo, that approval bound to the exact commit and its report's digest (`project/instrukt/spec/feature/work/stamps`). No pull-request review exists for it and none is cited. |
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

A computer that holds no evidence for an eligible head recognises the verdict
the reviewer bot published last for that commit. It reads the reviewer bot's
own publications for exactly that commit, each verified by the bot's identity.

| Read | Counts when |
| --- | --- |
| The reviewer bot's newest `instrukt/review` status on the commit | Its state is success or failure and its description carries a verdict's name and moment. |
| The review that status links, read by its own address | Its author is the reviewer bot, it is a review of exactly that commit in the project's repository, its native state is approving or change-requesting, and its body carries a verdict's name and moment. |
| The reviewer bot's latest review of exactly that commit on each eligible pull request the runner observes at that head, which is the one GitHub lists last there | The same as for the linked review. |

The recognised verdict is the one decided last among those these reads give:
its state, its name and its moment. Which was decided last is read from the
moments the publications carry and from nothing else. The date GitHub gives a
status or a review is the date of that delivery: a verdict delivered again, to
a further pull request or as a status written once more, is dated anew and was
not decided again, so GitHub's dates order no two verdicts. Publications that
carry one name and one moment carry one verdict. Of two verdicts the one with
the later moment was decided last. Where two moments are equal and the names
differ, the verdict that reports failure or requests changes counts as the
later, and between two of one state the one whose name sorts last. The order
follows from what the publications carry alone, so every tick recognises the
same verdict, whichever pull requests received it and in whatever order.

The linked review is read where it was written, whatever became of its pull
request: one that was closed without merging still gives it. A head with a
recognised verdict carries the reviewer identity's verdict and is asked for no
review, also when the status alone carries it.

Recognition keeps the verdict's state, name and moment and, where a review
that counts carries that name and moment, that review's body. Each destination
then follows the recognised verdict as it follows a current one. The status is
owed while the reviewer bot's newest one does not show that state, name and
moment, or links no review although a review that counts carries them, and is
written with them, linking such a review. An eligible pull request at that
head whose latest review by the reviewer bot of the commit does not show them
is owed a review of that state carrying that body as it was read. What
recognition writes repeats a verdict and decides none: it carries the
recognised name and moment unchanged and is given no moment of its own, and
the date GitHub gives it is the date of its delivery. A publication that
stopped between a review and its status is completed this way, a further pull
request at the head receives the same verdict with the same findings, and no
destination is written with a verdict decided earlier than the last one these
reads give, however lately the earlier one was delivered. The commit is judged
once.

Where no review that counts carries the recognised name and moment, this
computer has no report to deliver. It writes no review, the pull requests keep
what they show beside the commit's status, and `instrukt review show` gives
the review as owed and not deliverable from this computer.

Recognition is no evidence: it earns no candidate coverage and admits no
revision. What it writes repeats the reviewer bot's own latest publication and
judges nothing. A status or a review that does not count recognises nothing,
and a head with none that counts is owed its review.

### Review evidence storage

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review.db` | The head verdicts recorded on or imported to this computer, each with its versions, their reports and their moments, and with whether it stands here, was handed off by an export or was set aside by an import; exemptions; and imported derived approvals. | `instrukt review record`, `exempt`, `export` and `import` only: commands their caller waits on. | `state/review.lock` |
| `state/review-publication.db` | The derived approvals the tick recorded, with their coverage; each withdrawal of a derived approval, of one an import brought as well, with the commit found uncovered and when it was withdrawn; per current verdict and destination what GitHub shows of it, with the cause of an outstanding or refused one; and the verdicts recognised as already published, each with its state, name, moment and review body. | The tick only. | `state/review-publication.lock` |

No background work writes the evidence store and no command a caller waits on
writes the publication store, so no command waits behind a tick
(`sdlc/policy/state-storage`). Neither store takes the state database's writer
lock, and readers take no lock. The publication store refers to evidence by
commit and version and shares no transaction with the evidence store. What it
holds of GitHub is held with the name and the moment GitHub shows, so a
verdict whose moment an import moved is owed again at each destination. It is
read again when it is missing, and whether the integration
head is covered now is derived again. Two things in it are obtained from
nowhere else and are kept there until they are retired: the derived approval
of a candidate the canonical branch has since taken, and each withdrawal with
the commit found uncovered. Whether a derived approval was withdrawn is read
from there by every reader of a commit's current verdict. Which head verdict
of a commit is current is read from the evidence store alone.

A withdrawal is the record of what one tick did and why, and that tick's write
is its only copy. A later derivation answers another question: whether the
integration head of its own tick is covered by the evidence of its own tick.
That is another candidate once the integration branch has moved and other
evidence once a head was revised. No derivation returns that an approval was
withdrawn, when, or for which commit. The tick writes the withdrawal as the
act itself: the approval is withdrawn from that write on. It is history from
then and is retired like other history, and it stays on the computer whose
tick recorded it: an export carries no withdrawn approval and no withdrawal.
Losing a withdrawal loses that account and causes no publication: an approval
that reads as not withdrawn again and that GitHub does not show is derived
again before anything of it is written.

Evidence is current for as long as it can still admit something. The head
verdict that stands here is current, in its current version, while the
canonical branch does not contain the commit, whether or not a branch still
reaches it: the status GitHub keeps for the commit admits it wherever it is
proposed again. An exemption is current while the canonical branch does not
contain the commit and a branch of the project still does; an outstanding
publication until it is published or refused; a derived approval while its
candidate is the integration head and it was not withdrawn. Everything else
is history: replaced versions, withdrawn derived approvals, a verdict an
export handed off or an import set aside, evidence of a commit the canonical
branch contains, an exemption of a commit no branch reaches any more, and
completed publications. History is retired after a retention period. Current
evidence and outstanding publications are never retired by age, so the
evidence store keeps one current verdict for every reviewed commit that has
not reached the canonical branch. A current head verdict rests on no history:
its current version carries its own verdict, report, reviewing session,
pinned base and moment from the write that recorded or imported it, so
retiring the version it replaced, or a verdict it was chosen over, changes
nothing of it. The evidence store retires its history inside a recording
command and reads nothing of the publication store for it; the publication
store retires its history inside a tick.

### Evidence when the serving computer changes

Both stores belong to one computer. GitHub carries the two publications and no
evidence, so review evidence reaches another computer through an export and an
import of the evidence store and through nothing else. A transfer runs one
way: from the computer that writes the file to the one that imports it.

| Command | Contract |
| --- | --- |
| `instrukt review export --to <file>` | Writes one snapshot of the project's review evidence that stands on this computer: the current version of every head verdict that stands here, as it was recorded and with its moment; every current exemption; and every derived approval it keeps that was not withdrawn; with what is not yet published and what concerns commits GitHub does not have. It holds `state/review.lock`, so a recording command falls wholly before the snapshot or wholly after it. The file declares the digest of its content. It is complete on disk before the hand-off is committed, and with that commit the exported verdicts are handed off. |
| `instrukt review import <file>` | Takes in an export, holding `state/review.lock`, commit by commit. A file whose content does not match the digest it declares is refused whole. It removes nothing the importing computer holds. |
| `instrukt review import <file> --standing imported\|held` | The same, and states which verdict stands for every commit of which the file and this computer hold two different ones. |

A handed-off verdict is history on the exporting computer. That computer
publishes nothing further of it and admits no further version for its commit;
the refusal names the export and its door, which is recording on the computer
that imported the file, or importing here a file that carries the commit's
verdict. Evidence recorded after the snapshot is not part of it. A later
export carries what stands then: no verdict an earlier export handed off and
none an import set aside, so a file holds at most one verdict of a commit, the
one that stood where the file was written. From the hand-off until the file is
imported its verdicts stand on no computer. The file is imported on the
computer that serves next, or on the one that wrote it, where they stand
again.

An import takes the file's verdict of each commit and reads the verdict of
that commit that stands here.

| The file's verdict of a commit | This computer | Result |
| --- | --- | --- |
| Any | No verdict of the commit stands here: it holds none, or the one it holds was handed off | The file's verdict stands here. |
| The version that is current here: the same verdict with the same report digest | A verdict of the commit stands here | Nothing changes, except that the standing version takes the file's moment where that is the later one. |
| Another verdict or another report | A verdict of the commit stands here | Two verdicts of one commit. Without `--standing` the import is refused whole: nothing changes, and the refusal names every such commit and that option. With `--standing imported` the file's verdict stands here and the one held is set aside. With `--standing held` the one held keeps standing and the file's is set aside. |

An exemption or a derived approval in a file is added where none is held for
that commit; where one is held, nothing changes. A verdict and an exemption of
one commit never replace each other.

Which verdict stands is the importing operator's statement of which computer's
record the project was served from. Work chooses between no two reviews, and
no import changes a commit's current verdict without that statement. The
statement holds for the import it is made at. The same file imported again
with the same statement changes nothing further.

An import at which one verdict is set aside for another decides the commit's
verdict again. From that import on, the current version of the verdict that
stands here has a later moment than the current version of the one set aside
(Verdict publication). Both moments are read in the transaction that sets the
verdict aside: the file's from the file and the held one's from the evidence
store. Where the standing verdict's moment moved, its destinations are owed
with the new one, and what is published after the import is recognised ahead
of everything that was published of the verdict set aside, by this computer
and by one that holds no evidence. A standing verdict that already had the
later moment keeps it, and a destination that shows it is not written again.

A verdict that was set aside is kept as history: its versions and reports, its
reviewing session, where and when each was recorded, and the import and
statement that set it aside. `instrukt review show` gives it beside the one
that stands. It publishes nothing, earns no coverage, admits no revision and
is carried by no export.

Imported evidence is the importing computer's from then on. It keeps its
reviewing session and where and when it was recorded, counts like evidence
recorded there, and has every destination of its current verdicts outstanding
until GitHub is found to show them or they are written. Evidence of a commit
the importing repository does not hold is kept and is published once GitHub
has the commit.

Neither command needs the project to be opted in, waits for GitHub or takes
the publication lock. An export reads the derived approvals the tick recorded,
and which of them were withdrawn, from the publication store as every reader
of it does, without a lock. An import reads the file and the evidence store
alone. Both write the evidence store alone, under `state/review.lock`: an
import keeps there the derived approvals a file carried.

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
`project/instrukt/spec/feature/configuration/layered-configuration`. The credentials that observe CI and that reflect build
reviews are unchanged: a project that has not opted in needs none of these
permissions.

## Use cases

### UC-GA1: See bot authorship and receive an independent native review

```gherkin
Given a developer has configured distinct project, author and reviewer Apps
When an author role opens its implementation pull request and a reviewer role approves it
Then GitHub shows the author bot and the different reviewer bot
And issue and board maintenance shows the project bot
```

### UC-GA2: Recover reflected work written by the configured bots

### UC-GA3: See an automatic review's verdict where GitHub can enforce it

```gherkin
Given a developer's pull request head has been reviewed automatically
When the reviewer's verdict is published
Then the pull request shows the reviewer bot's approving or change-requesting review with its findings
And the reviewed commit carries the reviewer bot's matching success or failure review status
But a later head of that pull request carries no status from that verdict
```

## Known caveats

- GitHub App installation/permission availability and provider artifact/board
  capabilities still bound remote work; token issuance supplies no missing API.
- Human approval attribution remains the approvals contract. App configuration
  does not grant a bot permission to speak for an authorized human.

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
- A verdict recognised as already published is no evidence, and the head earns
  no candidate coverage on a computer that holds none. Recognition reads the
  reviewer bot's newest status, the review it links and the reviews on the
  pull requests observed at the head. A review on a pull request that is
  closed and that the status does not link is found by no read, so a later
  verdict that reached GitHub only there is not recognised.
- A verdict that the status alone carries is recognised without its report, as
  for a head reviewed before any eligible pull request had it. A computer
  without the evidence does not have the head reviewed again and writes no
  review for it: its pull requests show the commit's status and no findings
  until the evidence is imported.
- Recognition orders published verdicts by the moments they carry. Within one
  verdict that is the order of its versions, and between a verdict and one an
  import set aside for it that is the choice. Two verdicts that neither a
  revision nor an import put in order, two verdicts of one commit that no
  import has met together or a head verdict and a derived approval, carry
  moments read from the clocks of the computers that recorded them, and their
  order is as true as those clocks agree. Of two verdicts of one moment the
  order is not known, and the refusing one is taken as the later.
- A choice between two verdicts reaches GitHub with the publications that
  follow the import. Until the importing computer has written them, GitHub
  orders the two as before the import, and a computer that is lost in between
  takes the choice with it. The choice is kept with the verdict that was set
  aside and travels in no export: another computer that meets the two states
  it for itself.
- An export file is the only copy of its verdicts that can stand anywhere.
  A file lost before it is imported leaves them handed off on the computer
  that wrote it, where `instrukt review show` gives them for the retention
  period. The computer that serves next recognises what was published of them
  and has every other owed head reviewed again.
- Work keeps no account of where a handed-off verdict went. An earlier file
  imported on a computer that has since handed the commit's verdict on makes
  the file's version stand there, beside whatever stands where the verdict
  was handed to. Which file is the latest is the importing operator's
  knowledge. The two are met as two verdicts of one commit when a file of one
  computer reaches the other.
- A handed-off verdict is kept for the retention period. After it the
  exporting computer holds nothing of the commit and admits a verdict for it
  as for any commit that has none.
- A derived approval is derived again until GitHub shows it as the commit's
  status. A covering head's verdict that changes after the derivation of the
  tick that publishes it, or after the status is shown, withdraws nothing: the
  candidate keeps its approving status.
- A withdrawal of a derived approval, with the commit found uncovered, is
  given by `instrukt review show` on the computer whose tick recorded it and
  for the retention period. No later derivation gives it again, and it is in
  no export: a computer that serves next derives the candidate's coverage for
  itself and holds no account of an earlier withdrawal.
- A handed-over approval is admitted only from a conversation instrukt can
  still read and with a report that ends in the fixed verdict line. Work
  establishes that the named session returned that report for that head. That
  the result still stands is the entering session's statement after reading
  that conversation; work records it and verifies nothing of it.
- A revision offered in a turn that a review request for the same commit
  opened is refused; the reviewing session records it in another turn. The
  answer rests on the input that opened the turn, so a request that a runtime
  steers into a running turn is not seen by it
  (`project/instrukt/spec/feature/runner/pull-request-review`).
- While the runtime cannot say which input opened the turn a revision is
  offered in, the revision is refused and the earlier version stays current.
  GitHub keeps showing that version and enforces by it: where it approved, a
  required review check on the commit stays met until the reviewing session
  records the revision once the runtime answers. Only a session that a review
  request of the commit reached is asked that question.
- While a candidate's needs-work verdict is admitted and its failure status
  not yet published, GitHub still shows the earlier approving status and would
  accept the push; the candidate's failed result is what holds it back
  (`project/instrukt/spec/feature/workflow/integration-gate`).
- A verdict whose pinned base does not lie on the canonical branch covers
  nothing on the canonical branch.
- Work verifies a reviewer's independence only against the sessions the
  reviewed commits name. For a commit that names none it verifies nothing about
  its author.

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

- https://docs.github.com/en/rest/commits/statuses — the newest-first listing, and the description and target address a status carries.
- https://docs.github.com/en/rest/pulls/reviews — the listing in submission order, a review read by its own address, and the submission date GitHub sets at each delivery, which no request can set.
