---
description: Pull requests from a project's trusted authors have their current head independently reviewed without anyone asking, and the verdict reaches GitHub where branch protection can require it.
---
# Pull-Request Review — Spec

## What it is

A project that opts in has the current head of every open, non-draft pull
request from its trusted authors reviewed independently, on instrukt's own
initiative. Nobody has to launch the reviewer or chase a stalled pull request.
instrukt owns the trigger and the review state; GitHub carries the verdict and
enforces it. The reviewer bot publishes the verdict on the pull request and on
the exact reviewed commit under
`project/instrukt/spec/feature/work/github-apps`, where the canonical branch's
required status checks can demand it before the pull request merges.

Pull requests a todo's work lifecycle owns are outside this capability: their
build review is reflected on them by
`project/instrukt/spec/feature/work/github-record`, and their content reaches the
canonical branch through the integration candidate.

## Canonical fields

### Observation

Each runner tick observes the opted-in project repository's open pull requests
and their current heads, with conditional requests, under the same source
acquisition rules as the other GitHub sources in
`project/instrukt/spec/feature/runner/event-activation`. An unchanged repository
costs no rate quota, and a missed tick is caught by the next tick reading
current state. The tick is the only trigger: observation needs no public
address, and a webhook delivery starts no review.

The listing is read page by page until a short page ends it, each page under
its own conditional validator, and eligibility is decided from the complete
listing only. Whether an eligible head already carries a verdict is work's
answer, from the evidence this computer holds or from what the reviewer bot
already published (`project/instrukt/spec/feature/work/github-apps`, Verdict
publication); the runner asks and reads no verdict itself. The listing uses a
credential of the project identity scoped to it
(`project/instrukt/spec/feature/work/github-apps`, Readiness for automatic
review), issued only for an opted-in project.

### Observation store

The pages kept for this source are a derived copy of GitHub's answers and live
in a store of their own.

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review-observation.db` | The pages of the current pull-request listing, each with its conditional validator. | The tick's review observation only. | `state/review-observation.lock` |

No command a caller waits on writes this store, and no write to it takes the
state database's writer lock, so a pass that reads many pages delays no
command (`sdlc/policy/state-storage`, Write isolation follows contention). It
has the shape of the history store
(`project/instrukt/spec/feature/runner/history`, History store): one writer,
its own lock, readers without a lock. A page beyond the current listing is
removed in the same pass. The store holds nothing else, so removing it loses no
fact: rebuilding costs one unconditional request per page. The poll deferral is the one the other
GitHub sources already keep.

### Which heads are reviewed

| Pull request state | Outcome |
| --- | --- |
| Open, not draft, head branch in the project's repository, trusted author, current head carries no verdict from the configured reviewer identity | One review of that exact head. |
| Current head already carries the reviewer identity's verdict | None. |
| Draft | None until it is marked ready. |
| Owned by a todo's work lifecycle | None; that lifecycle reviews it. |
| Head branch in a fork | None. |
| Author outside the project's trusted authors | None. |
| Opened by an automated dependency-update bot | None; a failed dependency update is assessed under `project/instrukt/spec/feature/workflow/integration-gate`. |
| Opened by the reviewer identity's own bot | None; an independent review needs an identity other than the author's (`project/instrukt/spec/feature/work/github-apps`, Role selection). |

A trusted author is the repository's owner, a member of its organization, a
collaborator, or the bot of one of the project's configured GitHub Apps other
than the reviewer identity's, recognised by its verified bot identity under
`project/instrukt/spec/feature/work/github-apps`. A head is reviewed once, however
often it is observed. A new push is a new head and is reviewed in its own right;
an earlier head's verdict never stands in for it. The activation starts an
independent reviewer on that exact head under
`sdlc/procedure/lifecycle/work/review-commits`, with no involvement from the
authoring session.

Eligibility reads who opened the pull request and where its head branch lives,
and nothing about the branch's name: a trusted author's pull request is
eligible on any branch of the project's repository. A bot account is a trusted
author only as a configured App's verified bot identity, whatever association
GitHub reports for it; every other bot account, a dependency-update bot among
them, is outside the trusted authors. Whether a pull request is owned by a
todo's work lifecycle is work's answer
(`project/instrukt/spec/feature/work/github-record`); the runner asks and does
not derive it.

A change-requesting review reaches an authoring thread that watches the pull
request through its runtime's pull-request watch; the fix it pushes is a new
head and is reviewed automatically.

### Review requests

A commit is owed a review while it is the current head of an eligible pull
request and work answers that it carries no verdict
(`project/instrukt/spec/feature/work/github-apps`, Review evidence and Verdict
publication). The review is one judgment of that commit: eligible pull requests
that share a head are owed one review between them, and each receives the
outcome as its own pull-request review.

An owed commit is asked for with the fact `runner.review_needed`
(`project/instrukt/spec/domain/runner`).

| Part | Value |
| --- | --- |
| Subject | The pull request, `pull_request:<project>/<number>`. Of several eligible pull requests that share the head, the lowest-numbered. |
| Data | `pull_request`, the pull request's number; `head_branch` and `base_branch`, its branch names; `head`, the exact head commit; `base`, the pinned base commit; `request`, the request's number for that head commit. |
| Occurrence identity | The project, the head commit and the request number. |

The pinned base is where the reviewed change starts: a commit, fixed for the
request and computed in the project's repository after its remote was fetched
for this request. How a request's pinned base is selected is defined here and
nowhere else.

| The head | Pinned base |
| --- | --- |
| Shares history with the project's canonical branch | Their merge base. The branch a pull request targets does not move it: the review status belongs to the commit and admits it wherever it is proposed, so one review covers everything the head adds to the canonical branch, also when the pull request targets another branch and when several pull requests with different base branches share the head. |
| Shares none with the canonical branch | The merge base of the head with the base branch of the pull request the request is addressed to. The review covers what the head adds to that branch. |

The branch names beside the pinned base only describe the pull request. The
consumer reviews from the pinned base and work admits the verdict against it
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence). A head for which the repository gives neither merge base is not
asked for in that tick.

The fact is a state: while a pull request's request has not been selected for
handover, a request for a newer head of that pull request takes its place.

### One review per commit

Requests are counted per project and commit, whichever pull requests have that
commit as their head, and a commit has at most one live request: one that is
queued, selected or handed over.

| Standing | The request |
| --- | --- |
| Queued | Is emitted, and delivery has not selected it. It can be replaced, and it ends when its commit stops being owed. |
| Selected | Is frozen by delivery for handover, to a session that is not started yet or to one whose runtime has not accepted it. From here on nothing replaces or withdraws it (`project/instrukt/spec/feature/runner/procedure-event-delivery`). No session holds it yet. |
| Handed over | Was accepted by the runtime for a named session. That session holds the request. |
| Ended | Has one of the dispositions below. |

A request's standing is delivery's own record of it, so the two never disagree
about whether it will still be handed over.

| Disposition | When |
| --- | --- |
| Answered | A verdict for the commit was recorded. Every request of that commit ends with it and the commit is asked for no more. |
| No verdict | The turn that received the request ended with no verdict recorded. |
| Unconfirmed | The runtime accepted the handover and delivery found no evidence within its bound that it entered a turn (`project/instrukt/spec/feature/runner/procedure-event-delivery`). Whether its session will still take it up is not known. |
| Never handed over | No responsibility accepted it, a newer queued request for its subject took its place, its handover failed, or its commit stopped being owed while it was queued. |

A turn the runtime reports as interrupted is no disposition: delivery replays
it.

An ended request can still enter a turn: one selected before the verdict was
recorded, one that ended unconfirmed, and one delivery replays. What a session
does with a request for a commit that is already settled belongs to its
procedure (`sdlc/procedure/lifecycle/work/review-commits`), and what a verdict
offered then does belongs to work
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence).

A live request belongs to its commit, not to the pull request that made the
commit owed. A selected or handed-over request does not end when the commit
stops being owed, as when the pull request gets a new head, becomes a draft or
is closed. While a request is live no further request is emitted for that
commit, also when another pull request, or the same one again, makes the commit
owed in the meantime. Its verdict reaches every eligible pull request that has
the commit as its head when the verdict is published.

While a commit is owed and has no live request, the runner emits the next
request for it under the next number, addressed to the lowest-numbered eligible
pull request that has it as its head at that moment. Each further request for
one commit waits longer than the one before, up to a bound, so a reviewer that
cannot finish is not started in a loop and the commit is never given up.

Work reads one answer from the runner when it admits a verdict: the commit's
requests from delivery's own records (`project/instrukt/spec/feature/work/github-apps`,
Admission of review evidence).

| Asked | Answered from |
| --- | --- |
| A commit's requests: each one's number, its standing, the session it was handed over to, an ended request's disposition, whether an ended request that delivery selected is still to be handed over, and its pinned base | Delivery's own records. |

A request's records are kept while it is live, while its commit is still owed,
and while a session it was selected for can still take it up. Request standing
and live runtime activity do not control verdict admission.

### Placement and consumer

The request executes in the project's registered checkout on the activating
computer. Before a request is emitted the project's remote is fetched there, so
the exact head commit and its pinned base are present in that checkout's
repository; the runner changes no working tree for it.

The runner publishes the request's subject, data and placement. Which procedure
answers a request, under which role and identity, and how it reviews are that
procedure's published declaration and contract
(`project/instrukt/spec/feature/runner/procedure-subscription-routing`).

### Candidate requests

The same fact with the integration branch as its subject,
`repository_ref:<project>/origin/integrate`, asks for review of an integration
candidate. Its data names the candidate commit as `head`, the pinned base
commit as `base`, which is its merge base with the canonical branch and so the
canonical head the candidate is based on, and the request number as `request`, and its occurrence identity is the project,
the candidate commit and the request number. When a candidate owes a review is
workflow's decision
(`project/instrukt/spec/feature/workflow/integration-gate`); its requests take
the standings and dispositions above and repeat as above.

### Asking a held candidate's authors

When a commit the active candidate adds is not covered, what follows — which
authors are asked, that the integrator is told, and whether the candidate owes
its own review — is workflow's decision
(`project/instrukt/spec/feature/workflow/integration-gate`, Review coverage of
a candidate). The runner supplies one fact to that decision and carries out
the acts it calls for.

<!-- planned-change:thin-live-runtime-boundary -->
**The fact: whether the runtime holds a named session.** For each session an
uncovered commit names, as work reads the commit's authoring sessions
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence), the runner answers from one bounded read of the selected runtime
per distinct session and derivation
(`project/instrukt/spec/feature/runner/session-continuity`, Adapter
primitives).

| Named session | The runner's answer |
| --- | --- |
| Held | The selected runtime reports the session present, busy or waiting on a person included, and it is a thread instrukt sends input to. |
| Not held | The value is no session reference; or it names another runtime than the selected one; or the selected runtime reports it gone, unknown to it, archived or deleted; or it is an instrukt-owned thread, of any binding kind, whose native project is not its checkout's, which instrukt sends nothing (`project/instrukt/spec/feature/runner/session-continuity`, T3 realization of keyed dispatch). |
| Cannot say | The runtime cannot be reached or its answer is an observation error; or delivery holds an instrukt-owned thread's input until its changed or unverified native project is restored. |
<!-- change:thin-live-runtime-boundary -->
**The fact: whether the runtime holds a named session.** For each session an
uncovered commit names, as work reads the commit's authoring sessions
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence), the runner answers from one bounded read of the selected runtime
per distinct session and derivation
(`project/instrukt/spec/feature/runtime/adapter`, Universal live spine).

| Named session | The runner's answer |
| --- | --- |
| Held | The selected runtime reports the session present, busy or waiting on a person included, and it is a thread instrukt sends input to. |
| Not held | The value is no session reference; or it names another runtime than the selected one; or the selected runtime reports it gone, archived, or deleted; or it is an instrukt-owned thread, of any binding kind, whose native project is not its checkout's, which instrukt sends nothing (`project/instrukt/spec/feature/runner/session-continuity`, Keyed dispatch and generation succession). |
| Cannot say | The runtime cannot be reached or its answer is an observation error; or delivery holds an instrukt-owned thread's input until its changed or unverified native project is restored. |
<!-- /planned-change:thin-live-runtime-boundary -->

Nothing is asked of a session the runtime cannot say about, and no review of
the candidate is asked for while any uncovered commit's session is unanswered;
the next tick derives again, and the sessions left unanswered are reported to
the operator through `instrukt events status`
(`project/instrukt/design/architecture/outside-gate`).

**Asking an author.** Each session workflow names to be asked for an uncovered
commit is sent one ask, a runner-authored `[Instrukt Notification]`
(`general/policy/message-identity-headers`) delivered through
the session's input delivery
(`project/instrukt/spec/feature/runner/procedure-event-delivery`). The runner
renders it in one fixed wording from the project, the candidate and the
commit, and it names `general/policy/commits` and the two ways to decide:
`instrukt review exempt --commit <commit> --basis <basis>`, or asking for the
independent review of a head that contains the commit
(`project/instrukt/spec/feature/runner/requested-review`). It carries no text
any agent wrote.

The runner keeps an ask record for each commit and session it asks, as
approval attention keeps its own record beside the input it queues
(`project/instrukt/spec/feature/runner/session-continuity`, Approval
attention). The record and its input are written in one transaction, before
anything reaches the runtime; the record's key, states and retirement are the
state database's `review_asks`
(`project/instrukt/spec/feature/work/state-database`).

| Ask record state | Meaning for the gate |
| --- | --- |
| Queued | The ask waits in delivery. The session counts as asked; nothing more is sent. |
| Asked | The runtime accepted the ask, whether or not delivery later saw it enter a turn. Nothing more is sent for that commit and session. |
| Undelivered | Delivery ended the ask without the runtime accepting it, as when keyed succession fails a misplaced thread's inputs. The record keeps the cause; a later derivation that still names that session to be asked sends a new ask under the next attempt. |

An ask whose session the runtime reports gone before it is delivered stays
pending under delivery's rule: delivery retries it on each pass and logs every
failed attempt, and the record stays queued. The gate meanwhile reads that
session as not held. When the record is retired, its inputs that delivery has
not selected or that ended failed are removed, so nothing stale is sent later
(`project/instrukt/spec/feature/work/state-database`, `review_asks`).

| Ask record | Value |
| --- | --- |
| Nature | External bookkeeping: whether a commit's author was asked, through which input, and with what outcome. |
| Owner | The runner; written only by the tick's review pass, which queues it, takes its outcome from delivery and retires it. |
| Loss cost | A repeated external effect: one more ask per lost record. |
| Medium | A table of the state database, beside the input delivery it is queued with, so the record and its input commit together. |
| Write isolation | The state database's writer lock, held by the tick for each short transaction that queues, updates or retires asks, never while it reads the runtime or GitHub. |
| Scope | One computer: the sessions asked are its runtime's. A computer that takes over serving the project (`project/instrukt/spec/feature/work/github-apps`, Evidence when the serving computer changes) holds none of these records and asks afresh. |

**Telling the integrator.** For a held candidate the runner emits
`runner.candidate_held` (`project/instrukt/spec/domain/runner`) once every
uncovered commit's sessions are answered: the project, the candidate, and each
uncovered commit with the sessions it names and whether each was asked or no
author of ours can decide for it. Its occurrence identity is the project and
the candidate, so the event log appends it once, however often the candidate
is found held.

**Candidate requests.** While the gate calls for no review of the candidate,
its queued requests end (One review per commit); when it calls for one, the
request is emitted as under Candidate requests.

### Knowledge proposals in a company home

In a company home admitted for automatic review, instrukt carries the
company's knowledge proposals to review and integration. The observation's
fetch of the home prunes remote-tracking branches the remote no longer holds,
and instrukt considers only the heads of the home's remote-tracking branches,
never a branch that exists only on the computer. Whether a head is an
admitted proposal is the company context's answer
(`project/instrukt/spec/domain/company`, Knowledge proposal).

- **Opening.** For each such head that is an admitted proposal, is not
  contained in the remote-tracking canonical branch, and that no pull
  request, open or closed, has as its head, instrukt opens a pull request for
  its branch as the identity the `company-agent` role selects
  (`project/instrukt/spec/feature/work/github-apps`, Company repositories).
  The pull request is then reviewed as every eligible pull request is.
- **Handoff.** When an open pull request's current head is an admitted
  proposal, holds an approving verdict work answers is current and covers the
  canonical branch, and is not contained in the remote-tracking canonical
  branch, instrukt emits `runner.proposal_approved`, which wakes the
  company's standing integrator to integrate that exact head. No person
  merges it: the integrator is the one writer of the company's default branch
  (`general/policy/commits`).
- **Reports.** `instrukt events status` lists, with their doors, the
  unestablished entries the company context names and a proposal store whose
  condition is not usable, derived through the company context's lookups each
  time it runs, and raises each as an anomaly
  (`project/instrukt/spec/feature/runner/anomaly-notification`).

Opening proposal pull requests, and nothing else, needs the proposal
pull-request credential, whose readiness work answers: met, unmet with a
finding, or GitHub cannot be asked
(`project/instrukt/spec/feature/work/github-apps`, Company repositories). It is
checked where it enters (`sdlc/principle/design-fundamentals`, Fail Fast):

- **At entry.** `instrukt install` and `instrukt install --check` ask it for
  each company home admitted for automatic review, beside automatic review's
  readiness, and report an unmet answer or one GitHub cannot give as a
  finding with its repair door.
- **On each pass.** The review observation of a company home asks it once.
  Met opens pull requests as above. Unmet withholds only the opening of
  proposal pull requests, with a warning naming the finding and its door.
  GitHub cannot be asked withholds only the opening for that pass; the
  source's cause is recorded and a warning is logged, as for automatic
  review's readiness, and the next tick asks again.

Automatic review of the home's other pull requests, the handoff of heads
already approved and CI observation never wait on this credential. An opening
GitHub or the runtime cannot answer is warned and tried again on the next tick
(Availability). An opening GitHub refuses while the credential is met is a
defect of the part: a warning
carries GitHub's answer. Neither writes any state.

### Enablement

The capability is enabled per project with the project-layer setting
`review.automatic`. What admission requires and where it is refused belong to
`project/instrukt/spec/feature/configuration/layered-configuration` (Automatic
review enablement); what is verified about the two identities belongs to
`project/instrukt/spec/feature/work/github-apps` (Readiness for automatic
review).

| Project on this computer | What the runner does |
| --- | --- |
| Admitted for automatic review | Observes its pull requests on every tick, asks for the reviews that are owed and has work publish what is outstanding. |
| Opted in and not admitted here | Observes no pull requests, emits no review request and has nothing published. |
| Not opted in | Observes no pull requests, emits no review request and has nothing published. Reviews the project already receives are unaffected, and so is how its CI results are read. |

The capability does not depend on the project's todos module.

Requiring the review status on the canonical branch, with the reviewer bot as
its expected source, is the repository owner's administration act; instrukt
publishes the status and does not change branch protection.

### Availability

When GitHub or the runtime is unavailable, observation, activation and
publication retry until the service returns, and the unavailable service is
reported to the operator (`project/instrukt/design/architecture/outside-gate`).

## Use cases

### UC-RV1: A developer's pull request is reviewed without anyone asking

```gherkin
Given a developer's project has opted in to pull-request review
When they open a pull request from a branch in the project's repository that is ready for review
Then its current head is reviewed independently without anyone starting a reviewer
When they push a fix to that pull request
Then the new head is reviewed in its own right
But observing an already reviewed head again starts no second review
And a draft pull request is not reviewed until it is marked ready
```

### UC-RV2: A developer opts a project in to pull-request review

```gherkin
Given a developer wants their project's pull requests reviewed automatically
When they enable pull-request review with a reviewer identity that can publish reviews and commit statuses on the repository
Then the setting is admitted and the project's pull requests are observed
But when the reviewer identity cannot publish reviews or commit statuses there
Then enabling it is refused with a message naming the missing permission and its repair
```

### UC-RV3: A developer's reviewers are not spent on outside contributions

```gherkin
Given a developer's project has opted in to pull-request review
When a pull request arrives from a fork, from an author outside the project's trusted authors, or from a dependency-update bot
Then no review is started for it
```

## Known caveats

- An automatically activated reviewer has no authoring session to ask for
  intent the diff does not show; it relies on the pull request's description and
  commits.
- Every new head of every eligible pull request starts a review, except a head
  another head replaced before its request was selected for handover and that
  did not become current again.
- A request is addressed to one pull request. Other eligible pull requests that
  share its head receive the same outcome as their own pull-request review and
  are addressed no request of their own. How many sessions answer requests, and
  whether one session answers a pull request's later heads, follows the
  answering procedure's declared identity
  (`project/instrukt/spec/feature/runner/procedure-subscription-routing`).
- A reviewer that keeps ending its turn without a verdict is asked again at
  growing intervals for as long as the head stays current. The head carries no
  review status meanwhile and, where the status is required, cannot merge.
- A commit that becomes a head again while its earlier request is selected or
  handed over waits for that review, however long that turn runs, and starts
  no other.
- After an unconfirmed handover the commit is asked for again, and a request
  selected before a verdict was recorded is still handed over. Two requests of
  one commit that both enter a turn before any verdict exists are both
  reviewed.
- A pull request that targets another branch than the canonical one is
  reviewed for everything its head adds to the canonical branch, which can be
  more than the pull request shows as its change.
- A pull request on a history the canonical branch does not share is reviewed
  against its own base branch. Another pull request that shares its head and
  has a different base branch receives that same verdict. A head that shares
  no history with its own pull request's base branch has no commit to review
  from and is asked for no review.
- One computer observes a project's pull requests at a time. Another computer
  recognises what the reviewer bot already published and holds none of the
  first one's evidence until it is imported
  (`project/instrukt/spec/feature/work/github-apps`, Evidence when the serving
  computer changes); a review in flight elsewhere is invisible to it, and it
  asks for its own. Two computers admitted for one project at once each ask
  and publish.
- A pull request outside eligibility starts no review, so no review status is
  published on its account. The status belongs to the commit and answers only
  the review check. Where a canonical branch requires that check, it is unmet
  for that pull request's head while the reviewer bot's approving status on
  that commit is missing or unsuccessful, and the merge is blocked. It is met
  once the commit carries that approving status, as when an eligible pull
  request has the same head; every other check the branch requires still has
  to pass, and a failing one still blocks the merge.

## See Also

- [project/instrukt/spec/feature/runner/event-activation](event-activation.md)
<!-- planned:thin-live-runtime-boundary -->
- [project/instrukt/spec/feature/runtime/adapter](../runtime/adapter.md)
<!-- /planned:thin-live-runtime-boundary -->
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../workflow/integration-gate.md)
