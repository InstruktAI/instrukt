---
description: Pull requests from a project's trusted authors have their current head independently reviewed without anyone asking, and the verdict reaches GitHub where branch protection can require it.
delivered_by:
  - event-driven-pr-review-gate
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
listing only. For an eligible head that work holds no review evidence for, the
tick also reads the evidence records GitHub carries on that commit, every page,
and hands them to work, so evidence recorded on another computer is recognised
and starts no second review. For an active integration candidate it reads, the
same way, the evidence records of the commits workflow's coverage finding
depends on. What counts as evidence, and from which identity, is work's answer
(`project/instrukt/spec/feature/work/github-apps`, Evidence records on
GitHub). These reads use a credential of the project identity scoped to
reading pull requests, issued only for an opted-in project.

### Observation store

The pages kept for this source are a derived copy of GitHub's answers and live
in a store of their own.

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review-observation.db` | The pages of the current pull-request listing and the evidence-record pages of the commits the tick currently reads them for, each with its conditional validator. | The tick's review observation only. | `state/review-observation.lock` |

No command a caller waits on writes this store, and no write to it takes the
state database's writer lock, so a pass that reads many pages delays no
command (`sdlc/policy/state-storage`, Write isolation follows contention). It
has the shape of the history store
(`project/instrukt/spec/feature/runner/history`, History store): one writer,
its own lock, readers without a lock. A page beyond the current listing and a
commit whose evidence records are no longer read lose theirs in the same pass.
The store holds nothing else, so removing it loses no fact: rebuilding costs
one unconditional request per page. The poll deferral is the one the other
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
request and work recognises no verdict for it: neither a head verdict nor a
derived approval (`project/instrukt/spec/feature/work/github-apps`, Review
evidence). The review is one judgment of that commit: eligible pull requests
that share a head are owed one review between them, and each receives the
outcome as its own pull-request review.

An owed commit is asked for with the fact `runner.review_needed`
(`project/instrukt/spec/domain/runner`).

| Part | Value |
| --- | --- |
| Subject | The pull request: the project and the pull request's number. Of several eligible pull requests that share the head, the lowest-numbered. |
| Data | The pull request's number, head branch and base branch, the exact head commit, the pinned base commit and the request's number for that head commit. |
| Occurrence identity | The project, the head commit and the request number. |

The pinned base is where the reviewed change starts: the merge base of the head
commit and the pull request's base branch, computed in the project's repository
after its remote was fetched for this request. It is a commit, fixed for the
request, and the branch names beside it only describe the pull request. The
consumer reviews from it, work admits the verdict against it and the recorded
evidence names it (`project/instrukt/spec/feature/work/github-apps`, Admission
of review evidence). A head whose merge base the repository cannot give is not
asked for in that tick.

The fact is a state: while a pull request's request has not been handed to a
session, a request for a newer head of that pull request takes its place.

### One review per commit

Requests are counted per project and commit, whichever pull requests have that
commit as their head, and a commit has at most one request that is queued or in
flight.

| A request is | From | Until |
| --- | --- | --- |
| Queued | It is emitted. | It is handed to a session or ends. |
| In flight | The runtime accepted its handover to a session. | Its outcome is known. |

| Outcome | What follows |
| --- | --- |
| A verdict for the commit is recorded. | Every request of that commit is closed and the commit is asked for no more. |
| The turn that received the request ended with no verdict recorded. | The request ends. |
| The handover stays unconfirmed: the runtime accepted it and delivery found no evidence within its bound that it entered a turn (`project/instrukt/spec/feature/runner/procedure-event-delivery`). | The request ends as unconfirmed. Whether its session will still take it up is not known. |
| The request never reached a session: no responsibility accepted it, a newer queued request for its subject took its place, its handover failed, or its commit stopped being owed while it was queued. | The request ends. |

A turn the runtime reports as interrupted is no outcome: delivery replays it.

An in-flight request belongs to its commit, not to the pull request that made
the commit owed. It does not end when the commit stops being owed, as when the
pull request gets a new head, becomes a draft or is closed. While it is in
flight no further request is emitted for that commit, also when another pull
request, or the same one again, makes the commit owed in the meantime. Its
verdict reaches every eligible pull request that has the commit as its head
when the verdict is published.

While a commit is owed and has no queued or in-flight request, the runner emits
the next request for it under the next number, addressed to the lowest-numbered
eligible pull request that has it as its head at that moment. Each further
request for one commit waits longer than the one before, up to a bound, so a
reviewer that cannot finish is not started in a loop and the commit is never
given up.

Every request of a commit asks for the same review. A session that was handed
one of them may record the verdict, also after that request ended or the pull
request moved on, and the verdict is that commit's alone. Work settles the
commit on the first verdict it admits and refuses a second judgment from the
holder of another request (`project/instrukt/spec/feature/work/github-apps`,
Admission of review evidence). A commit asked for again after an unconfirmed
handover therefore ends with one verdict, also when the first handover enters a
turn after all.

Work reads a commit's requests from the runner when it admits a verdict: each
request's number, whether it is queued, was handed to a session and to which,
or ended before it reached one, and its pinned base. A request's records are
kept while it is queued or in flight and while its commit is still owed.

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

The same fact with the integration branch as its subject asks for review of an
integration candidate. Its data names the candidate commit, the pinned base
commit, which is the canonical head the candidate is based on, and the request
number, and its occurrence identity is the project, the candidate commit and
the request number. When a candidate owes a review is workflow's decision
(`project/instrukt/spec/feature/workflow/integration-gate`); its requests are
queued, in flight, end and repeat as above.

### Enablement

The capability is enabled per project with the project-layer setting
`review.automatic`. What admission requires and where it is refused belong to
`project/instrukt/spec/feature/configuration/layered-configuration` (Automatic
review enablement); what is verified about the two identities belongs to
`project/instrukt/spec/feature/work/github-apps` (Readiness for automatic
review).

| Project on this computer | What the runner does |
| --- | --- |
| Admitted for automatic review | Observes its pull requests on every tick, asks for the reviews that are owed and has outstanding review evidence published. |
| Not opted in, or opted in and not admitted | Observes no pull requests, emits no review request and has nothing published. Reviews the project already receives are unaffected, and so is how its CI results are read. |

The capability does not depend on the project's todos module.

Requiring the review status on the canonical branch, with the reviewer bot as
its expected source, is the repository owner's administration act; instrukt
publishes the status and does not change branch protection.

### Availability

When GitHub or the runtime is unavailable, observation, activation and
publication retry until the service returns, and the unavailable service is
reported to the operator (`project/instrukt/policy/checks-and-guards`).

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
  another head replaced before its request was handed to a session and that did
  not become current again.
- A request is addressed to one pull request. Other eligible pull requests that
  share its head receive the same outcome as their own pull-request review and
  are addressed no request of their own. How many sessions answer requests, and
  whether one session answers a pull request's later heads, follows the
  answering procedure's declared identity
  (`project/instrukt/spec/feature/runner/procedure-subscription-routing`).
- A reviewer that keeps ending its turn without a verdict is asked again at
  growing intervals for as long as the head stays current. The head carries no
  review status meanwhile and, where the status is required, cannot merge.
- A commit that becomes a head again while its earlier request is still in
  flight waits for that review, however long that turn runs, and starts no
  other.
- After an unconfirmed handover the commit is asked for again, so two sessions
  can come to review one commit. Work admits one verdict; the other session's
  review is spent.
- One computer observes a project's pull requests at a time. Another computer
  recognises review evidence once it is reflected on GitHub
  (`project/instrukt/spec/feature/work/github-apps`); a review in flight
  elsewhere is invisible to it, and it asks for its own.
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
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../workflow/integration-gate.md)
- [project/instrukt/policy/checks-and-guards](../../../policy/checks-and-guards.md)
