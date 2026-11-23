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
| `instrukt review record --head <sha> --base <sha> --verdict approve --report <file\|-> --reviewer <session>` | Records the approving head verdict that the named session reached, entered by the calling session as that session's standing result (A handed-over approval). |
| `instrukt review exempt --commit <sha> --basis <text>` | Records an exemption for a commit the canonical branch does not contain yet. |
| `instrukt review show <sha>` | What this computer holds for the commit: its settlement with its identity, its hand-off number and its verdict versions; a pending revision with the moment it was offered; every pending revision that was decided, with the answer that decided it, as the version it became or as the offer that was refused; its exemption and its derived approval, or a derived approval the tick withdrew with the commit found uncovered; each of these with its pinned base and where it was recorded; a settlement an import set aside, with a revision that was pending on it; every publication with its state and the cause of an outstanding or refused one; and a verdict recognised as already published where this computer holds none. |
| `instrukt review export --to <file> --for <computer>` | Hands the project's review evidence held on this computer off into one file for the named computer (Evidence when the serving computer changes). |
| `instrukt review import <file> [--standing imported\|held]` | Takes in an export written for this computer (Evidence when the serving computer changes). |

The first verdict admitted for a commit settles its review and starts the
commit's settlement: that verdict with every later version of it. A settlement
has an identity, given when it starts and unique among all computers. A later
version exists only as a revision: the session that recorded the current
version judges its own findings again, as when the author disputes them with
evidence and no new commit. The latest version is current and earlier ones are
kept as history.

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
| Range | The pinned base is fixed by instrukt and the caller cannot narrow it. A request of the commit, live or ended, carries the pinned base the runner selected for it (`project/instrukt/spec/feature/runner/pull-request-review`, Review requests). For a commit that has had no request, work pins the head's merge base with the canonical branch, as `sdlc/procedure/lifecycle/work/review-commits` pins its candidate, and refuses a head that has none. A verdict whose pinned base does not lie on the canonical branch is recorded as not covering that branch. A later version of a settled commit keeps the pinned base of the version it replaces. `--base` states where the review started and is admitted when it is the pinned base or one of that base's ancestors. | The pinned base. |
| Independence | No commit after the pinned base up to the head names the reviewing session in its `Session:` trailer. | The commits that name the reviewing session. The door is a review by a session none of them names. |
| Settlement | The commit has no verdict yet, or the verdict is a revision: the caller is the session that recorded the current version, the commit has no pending revision, and, where a review request for this commit reached that session, the turn the session had in progress when it offered the verdict was not opened by such a request. | The settled verdict and `instrukt review show`; for the session that recorded it, which of the answers below applied, or the pending revision. |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

A review request is answered in the turn it opened. A verdict the settling
session offers in such a turn, for a commit that is already settled, answers
that request a second time. Only a session that a request of the commit
reached can be in such a turn, so work reads the commit's requests first
(`project/instrukt/spec/feature/runner/pull-request-review`, One review per
commit).

| The commit's requests | The settling session's offer |
| --- | --- |
| None was handed over to that session, and none that delivery selected is still to be handed over | Is admitted as a revision from the stores alone. The reviewer of a commit that never had a request is always in this row. |
| One was handed over to that session, or one that delivery selected is still to be handed over | Is decided by which input opened the turn the session had in progress when it offered the verdict. |

Which input opened that turn is the runner's answer, asked for the moment the
verdict was offered. Every other verdict is admitted or refused from the
stores alone.

| The runner answers | The offered verdict |
| --- | --- |
| Something other than a review request for this commit opened that turn | Is admitted as a revision. |
| A review request for this commit opened that turn | Is refused; no version is recorded. |
| The session had no turn in progress at that moment | Is refused; a revision is recorded within a turn of its session. |
| The runtime cannot say | Is asked for again until the command's own wait has passed. Then the offer is kept as a pending revision, and the command ends with an outcome of its own that is neither recorded nor refused. Its caller has nothing to repeat. |

The command that keeps a pending revision ends with an exit status of its own,
distinct from a recorded verdict and from a refusal. Its answer states that no
version is recorded and that the revision is offered and undecided, names the
commit and the offer by the digest of its report, and names
`instrukt review show <sha>` as where the decision is read.

A pending revision is the offered verdict with its report, its session and the
moment it was offered, stored in the evidence store by the command that
received it. It belongs to the settlement it would revise and is not a version
yet. While a commit's standing settlement has one, the commit is undecided:

- no review is asked for the commit, and a further verdict offered for it is
  refused and names the pending revision;
- nothing is published for the commit, so no destination is written with the
  earlier version;
- work answers the commit as settled and undecided, neither approved nor
  refused. What a candidate's coverage and its promotion make of an undecided
  commit is workflow's
  (`project/instrukt/spec/feature/workflow/integration-gate`, Candidate and
  verification; Review coverage of a candidate);
- what GitHub already shows stays, and GitHub enforces by it. A status the
  reviewer bot published for the earlier version still answers a required
  check on the commit, at a pull request and at a push to the canonical
  branch, until the revision is decided and, when admitted, published. Work
  writes nothing to GitHub for an offer that is no version;
- an export of this computer's evidence is refused (Evidence when the serving
  computer changes).

On each tick of the computer that holds it, work asks the runner the same
question for the same session and the same moment, for the pending revision of
each standing settlement, and the runtime is reported to the operator for as
long as it cannot say (`project/instrukt/policy/checks-and-guards`). The first
of the other three answers decides the pending revision as the table says.
Admitted, it is the commit's current version, recorded with the moment it was
offered, and is published. Work answers the commit by that version from its
admission, before anything of it is published. Refused, it is kept as history with the answer that
refused it, and the earlier version is published as current again. The
question names the moment of the offer, so a turn the session starts or ends
afterwards changes nothing about the answer. A pending revision of a
settlement that an import set aside is decided by no tick; it stays with that
settlement as it was offered.

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
(`project/instrukt/spec/feature/workflow/integration-gate`, Review coverage of
a candidate).

| Workflow finds again | The derived approval GitHub does not show yet |
| --- | --- |
| Every commit covered | Is published, with the coverage of this finding. |
| A commit not covered | Is withdrawn. The tick records the withdrawal with the commit found uncovered, and the candidate carries no verdict. |
| Nothing, the repository being unable to answer or a head being undecided | Stays recorded and unpublished in that tick. |

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
(`project/instrukt/spec/feature/configuration/layered-configuration`, Automatic
review enablement), whether or not that project's todos module is on. It
publishes the current verdict of each commit this computer holds evidence for:
the current version of its head verdict, or its derived approval. An exemption
publishes nothing, evidence this computer handed off is not published from it,
nothing is published for a commit while it has a pending revision, and a
derived approval that GitHub does not show yet is published only by a tick
that found its coverage again (Admission of review evidence).

A version is named by the digest of its report, and a derived approval by the
digest of its coverage. Each verdict also has its moment, which says when it
was decided and never when it was published. A head verdict is decided when
it is recorded, and decided again when an import keeps or makes its settlement
the commit's over another one. Its moment is later than the moment of the
verdict it was decided over.

| Decided by | The moment of the version |
| --- | --- |
| Its recording | The moment it was recorded with, for an admitted revision the moment it was offered. Where that is not later than the moment of the version it replaces, it is the first moment after that one. |
| An import that sets a settlement aside for the one that stands on this computer (Evidence when the serving computer changes) | Unchanged where the standing settlement's current version already has a later moment than the latest version of the settlement set aside. Otherwise that current version takes the moment of the import, or the first moment after the set-aside one's where the import's is not later. |

The moments of a commit's head verdicts thus keep the order in which they
became current, across the versions of a settlement and across a settlement
and one an import set aside for it, whatever the clocks of the computers that
held them read. An import moves the moment of no other version and changes
nothing else of one: when and where a version was recorded stay with it, and
`instrukt review show` gives both beside its moment. A derived approval's
moment is the moment the tick recorded the coverage it is published with. The
moment is kept with the verdict and travels with it through an export and an
import, and a publication carries the moment its verdict has when it is
written. The status carries the name and the moment in its description, and
the review carries both at the head of its body, ahead of the report or the
coverage.

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
of it with the earlier one and over what it shows of the settlement set aside.
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
| `state/review.db` | Settlements as recorded on or imported to this computer, each with its identity, its hand-off number, the settlements it was chosen over and its verdict versions with their reports; pending revisions; exemptions; imported derived approvals; for a settlement this computer handed off, the computer it was handed to and the moment its current version had when it was handed off; and the settlements an import set aside. | `instrukt review record`, `exempt`, `export` and `import` only: commands their caller waits on. | `state/review.lock` |
| `state/review-publication.db` | The derived approvals the tick recorded, with their coverage; each withdrawal of a derived approval, of one an import brought as well, with the commit found uncovered and when it was withdrawn; the answer that decided each pending revision; per current verdict and destination what GitHub shows of it, with the cause of an outstanding or refused one; and the verdicts recognised as already published, each with its state, name, moment and review body. | The tick only. | `state/review-publication.lock` |

No background work writes the evidence store and no command a caller waits on
writes the publication store, so no command waits behind a tick
(`sdlc/policy/state-storage`). Neither store takes the state database's writer
lock, and readers take no lock. The publication store refers to evidence by
commit and version and shares no transaction with the evidence store. What it
holds of GitHub is held with the name and the moment GitHub shows, so a
verdict whose moment an import moved is owed again at each destination. It is
read again when it is missing, and whether the integration
head is covered now is derived again. Three things in it are obtained from
nowhere else and are kept there until they are retired: the derived approval
of a candidate the canonical branch has since taken, the answer that decided a
pending revision, and each withdrawal with the commit found uncovered. Whether
a pending revision is decided, and how, and whether a derived approval was
withdrawn, is read from there by every reader of a commit's current verdict.

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

Evidence is current for as long as it can still admit something. A commit's
current verdict is current while the canonical branch does not contain the
commit, whether or not a branch still reaches it: the status GitHub keeps for
the commit admits it wherever it is proposed again. A pending revision is
current until it is decided or set aside with its settlement. What this
computer knows of each settlement of a commit, its identity, the highest
hand-off number it has met it at, as its holder, as a former holder or as one
it set aside, and the settlements it was chosen over, is current for as long
as a verdict of that commit would be. So are two things this computer keeps of
a settlement it handed off: the computer the settlement was handed to, which
the refusals of a further version and of an import that meets the settlement
name, and the moment the settlement's current version had when it was handed
off, which an import that sets the settlement aside compares (Evidence when
the serving computer changes). Both are kept with the settlement and not with
its versions, so retiring those versions removes neither. An
exemption is current while the canonical
branch does not contain the commit and a branch of the project still does; an
outstanding publication until it is published or refused; a derived approval
while its candidate is the integration head and it was not withdrawn.
Everything else is history:
replaced versions, refused offers, withdrawn derived approvals, the versions and reports of a settlement
an export handed off or an import set aside, evidence of a commit the
canonical branch contains, an exemption of a commit no branch reaches any more, and completed
publications. History is retired after a retention period. Current evidence
and outstanding publications are never retired by age, so the evidence store
keeps one current verdict for every reviewed commit that has not reached the
canonical branch. The evidence store retires its history inside a recording
command, the publication store inside a tick.

### Evidence when the serving computer changes

Both stores belong to one computer. GitHub carries the two publications and no
evidence, so review evidence reaches another computer through an export and an
import of the evidence store and through nothing else. A transfer runs from one
computer to one named computer.

| Command | Contract |
| --- | --- |
| `instrukt review export --to <file> --for <computer>` | Writes one cut of the project's review evidence on this computer for the computer named by its instrukt machine ID: every standing settlement with its identity, its hand-off number, the settlements it was chosen over and all its versions and reports, every current exemption and every derived approval it keeps that was not withdrawn, with what is not yet published and what concerns commits GitHub does not have. It holds `state/review.lock`, so a recording command falls wholly before the cut or wholly after it. The file names the computer that wrote it and the computer it is for. It is complete on disk before the cut is committed, and with the cut the exported settlements are handed off to that computer. |
| `instrukt review import <file>` | Takes in an export written for this computer, holding `state/review.lock`, commit by commit. On any other computer it is refused whole, and the refusal names the computer the file is for. It removes nothing the importing computer holds. |
| `instrukt review import <file> --standing imported\|held` | The same, and states which settlement stands for every commit of which the export and this computer hold two different ones. |

An export leaves nothing out. While a standing settlement on this computer has
a pending revision, the export is refused whole and writes nothing. The
refusal names each such commit and its door: a tick of this computer decides
the revision once the runtime answers, and `instrukt review show <sha>` gives
its state. A pending revision is decided only where its session's turn is
recorded, so a transfer is complete only when none is left to decide.

A settlement keeps its identity on every computer it reaches, and it carries a
hand-off number: zero where it started and one higher with each export that
hands it off. The number counts the hand-offs of that one settlement. It is
compared only with a number of the same settlement and orders nothing between
two settlements. The export writes the raised number into the file, and the
exporting computer keeps it with the computer the settlement was handed to. A
handed-off settlement is history on the exporting computer. That computer
publishes nothing further of it and admits no further version for its commit;
the refusal names the export and its door, which is recording on the computer
the settlement was handed to or importing an export that computer wrote for
this one. Evidence recorded after the cut is not part of it. A later export
for the same computer carries it, and carries again, at their unchanged
hand-off numbers, the settlements earlier exports handed to that computer for
as long as they are kept, so a lost file loses no evidence. An export for
another computer carries none of them. A settlement an import set aside on
this computer is carried by no export, so an export holds at most one
settlement of a commit.

An import takes the export's settlement of each commit and reads two things
this computer holds: whether it has met that settlement before, as its holder,
as a former holder or as one it set aside, and at which hand-off number; and
the settlement of the commit it holds in place, standing or handed off. A
computer holds at most one settlement of a commit in place; every other one it
keeps was set aside. A settlement records another when a choice of it over
that one was recorded on it at an import.

| The export's settlement of a commit | The settlement this computer holds in place | Result |
| --- | --- | --- |
| Was met here at the export's hand-off number or a higher one, as standing, as handed off or as set aside | Any, or none | An earlier transfer of that settlement: nothing changes, and the import names it. A file imported a second time, and one that returns to a computer which has since handed the settlement on, make nothing current. |
| Was not met here at that number | None, or this same settlement at a lower number | The export's versions in their order; the latest is current here, at the export's hand-off number. |
| Was not met here at that number | Another, which the export's settlement records, and which does not stand here recording the export's | The choice between the two was stated at an earlier import and travels with the settlement that stands. The export's versions are current here at the export's hand-off number and the one held is set aside. No statement is asked for again. |
| Was not met here at that number | Another, which records the export's settlement while the export's does not record it | The same choice, met from its other side. The export's settlement is set aside at its hand-off number and the one held keeps its place, standing or handed off. No statement is asked for again. |
| Was not met here at that number | Another, where neither records the other, or each records the other and the one held stands here | Two settlements of one commit, whatever their hand-off numbers. Without `--standing` the import is refused whole: nothing changes, and the refusal names every such commit, that option and, for a settlement held as handed off, the computer it was handed to. With `--standing imported` the export's versions are current here at the export's hand-off number, the one held is set aside, and the export's settlement records it. With `--standing held`, where the one held stands here, it keeps its place, the export's is set aside at its hand-off number, and the one held records the export's. |

`--standing held` is admitted only for a settlement that stands on this
computer. Where the settlement held in place was handed off, an import with
`--standing held` is refused whole and changes nothing: a choice is recorded
on a settlement that stands where it is stated, and a handed-off settlement
stands on another computer or on none. The refusal names each such commit, the
computer its settlement was handed to and both doors: `--standing imported` on
this computer, which records the choice on the export's settlement; or the
choice stated on the computer the settlement was handed to, at an import there
that meets both settlements, after which that computer's export for this one
brings the chosen settlement with its choice.

An exemption or a derived approval in an export is added where none is held
for that commit; where one is held, nothing changes. A verdict and an
exemption of one commit never replace each other.

Which settlement stands is the importing operator's statement of which
computer's record the project was served from. Work chooses between no two
reviews, and no import changes a commit's current verdict without that
statement, made at this import or recorded on the settlement an earlier one
chose. A settlement chosen over another is thereby chosen over every
settlement that one records. Every choice an import accepts is recorded on a
settlement that stands on the importing computer when the import is done, and
that settlement carries its recorded choices in every later export. A computer
that holds either settlement of a recorded choice therefore takes the chosen
one, or sets the other aside, without a second statement. Where each of two
settlements records the other, two statements contradict: a settlement that
stands here is not displaced without a new statement, and one that was handed
off gives way to the settlement an export brings.

An import that sets a settlement aside decides the commit's verdict again,
whether the choice is stated at it or was recorded at an earlier one. From
that import on, the current version of the settlement that stands on this
computer has a later moment than every version of the one set aside (Verdict
publication), and its destinations are owed with that moment. What is
published after the import is therefore recognised ahead of everything that
was published of the settlement set aside, by this computer and by one that
holds no evidence. A settlement that keeps its place as handed off stands on
another computer and is not changed here.

Where the settlement set aside is one this computer handed off, the moment
compared is the one this computer keeps of it: the moment its current version
had when it was handed off (Review evidence storage). The import
reads it from the evidence store, so it decides the same before and after that
settlement's versions were retired, and it reads nothing but the evidence
store and the export.

Setting a settlement aside takes a revision that is pending on it along. A
settlement that was set aside is kept whole: its identity, versions, reports
and reviewing sessions, where and when each was recorded, that pending
revision, and the import and choice that set it aside. `instrukt review show` gives it beside the standing one. It
publishes nothing, earns no coverage and admits no revision, and its pending
revision is decided by no tick.

Imported evidence is the importing computer's from then on. It keeps its
reviewing session and where and when it was recorded, counts like evidence
recorded there, and has every destination of its current verdicts outstanding
until GitHub is found to show them or they are written. A file whose content
does not match the digest it declares is refused whole. Evidence of a commit
the importing repository does not hold is kept and is published once GitHub
has the commit.

Neither command needs the project to be opted in, waits for GitHub or takes
the publication lock: an export reads the derived approvals and the decided
revisions as every reader of the publication store does, and an import keeps
them in the evidence store.

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
  settlement that is the order of its versions, and between a settlement and
  one an import set aside for it that is the choice. Two verdicts that neither
  a revision nor an import put in order, two settlements of one commit that no
  import has met together or a head verdict and a derived approval, carry
  moments read from the clocks of the computers that recorded them, and their
  order is as true as those clocks agree. Of two verdicts of one moment the
  order is not known, and the refusing one is taken as the later.
- A choice between two settlements reaches GitHub with the publications that
  follow the import. Until the importing computer has written them, GitHub
  orders the two settlements as before the import, and a computer that is lost
  in between takes the choice with it. A moment is moved past the set-aside
  settlement as the importing computer holds it: where that settlement was
  handed off and is revised further on the computer it was handed to, a
  version published from there carries a later moment again, until that
  computer imports the chosen settlement.
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
  (`project/instrukt/spec/feature/runner/pull-request-review`, One review per
  commit).
- While the runtime cannot say which input opened the turn a revision was
  offered in, the revision is pending and its commit undecided. GitHub keeps
  showing the earlier version and enforces by it: where that version
  approved, a required review check on the commit stays met on GitHub until
  the revision is decided and published. Only a session that a review request
  of the commit reached can have a pending revision. It is decided on a tick
  of the computer that holds it; where no tick runs for the project, or the
  runtime does not answer, it stays pending and that computer's evidence
  cannot be exported.
- A settlement is handed off to one computer at a time, and a choice between
  two settlements of one commit is recorded where it was stated. A computer
  that has not imported the chosen settlement keeps what it holds. A computer
  that handed its settlement off cannot choose it at an import: that choice is
  stated where the settlement stands.
- While a candidate's needs-work verdict is admitted and its failure status
  not yet published, GitHub still shows the earlier approving status and would
  accept the push; the candidate's failed result is what holds it back
  (`project/instrukt/spec/feature/workflow/integration-gate`, Candidate and
  verification).
- A verdict whose pinned base does not lie on the canonical branch covers
  nothing on the canonical branch.
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

<!-- planned:event-driven-pr-review-gate -->
- https://docs.github.com/en/rest/commits/statuses — the newest-first listing, and the description and target address a status carries.
- https://docs.github.com/en/rest/pulls/reviews — the listing in submission order, a review read by its own address, and the submission date GitHub sets at each delivery, which no request can set.
<!-- /planned:event-driven-pr-review-gate -->
