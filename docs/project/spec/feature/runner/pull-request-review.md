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
listing only. For an eligible head that work holds no verdict for, the tick
also reads that commit's statuses and that pull request's reviews, every page,
so a verdict the reviewer identity published from another computer is
recognised and starts no second review. What counts as that identity's verdict
is work's answer (`project/instrukt/spec/feature/work/github-apps`). These
reads use a credential of the project identity scoped to reading pull requests
and commit statuses, issued only for an opted-in project.

The validators kept for this source are a derived copy of GitHub's answers:
the pages of the current listing, and the status and review reads of current
eligible heads without a recorded verdict. A page beyond the current listing
and a head that is no longer current lose theirs in the same pass. Rebuilding
them costs one unconditional request per listing page and one per page of each
such head's statuses and reviews.

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
collaborator, or the bot of one of the project's configured GitHub Apps,
recognised by its verified bot identity under
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
request and no verdict for it is recognised. The review is one judgment of
that commit: eligible pull requests that share a head are owed one review
between them, and each receives the verdict as its own pull-request review
(`project/instrukt/spec/feature/work/github-apps`).

An owed commit is asked for with the fact `runner.review_needed`
(`project/instrukt/spec/domain/runner`).

| Part | Value |
| --- | --- |
| Subject | The pull request: the project and the pull request's number. Of several eligible pull requests that share the head, the lowest-numbered. |
| Data | The pull request's number, head branch and base branch, the exact head commit and the request's number for that commit. |
| Occurrence identity | The project, the head commit and the request number. |

The fact is a state. While a pull request's request is still undelivered, a
request for a newer head of that pull request takes its place. A request that
reached its reviewer stays that commit's: its verdict is accepted after the
pull request has moved on, and names that commit alone.

A request is open until a verdict for its commit is recognised or the commit
is no longer owed. It ends without a verdict when no responsibility accepted
it, when it was superseded or failed before it reached a reviewer, or when the
reviewer's turn that received it ended with no verdict recorded. A turn the
runtime reports as interrupted has not ended; delivery replays it
(`project/instrukt/spec/feature/runner/procedure-event-delivery`). While the
commit is still owed after its request ended, the runner emits the next
request for it under the next number, also when the commit became current
again after another head had taken its place. Each further request for one
commit waits longer than the one before, up to a bound, so a reviewer that
cannot finish is not started in a loop and the commit is never given up. A
commit with a recognised verdict is not asked for again.

The subscribed responsibility has subject identity
(`project/instrukt/spec/feature/runner/procedure-standing-agent`): one reviewer
per pull request receives each of its later heads. The request executes in the
project's registered checkout on the activating computer. Before a request is
emitted the project's remote is fetched there, so the exact head commit is
present in that checkout's repository; the runner changes no working tree for
it. Which procedure answers the request, and how it reviews, belong to the
published subscription and its procedure
(`project/instrukt/spec/feature/runner/procedure-subscription-routing`).

The same fact with the integration branch as its subject asks for review of an
integration candidate. Its data names the candidate commit, the canonical head
it is based on and the request number, and its occurrence identity is the
project, the candidate commit and the request number. When a candidate owes a
review is workflow's decision
(`project/instrukt/spec/feature/workflow/integration-gate`); its requests open,
end and repeat as above.

### Enablement and prerequisites

The capability is enabled per project with the project-layer setting
`review.automatic`
(`project/instrukt/spec/feature/configuration/layered-configuration`). For a
project that has not opted in it observes no pull requests, starts no reviews
and publishes nothing; reviews the project already receives are unaffected, and
so is how its CI results are read. It does not depend on the project's todos
module.

Opting in requires the configured reviewer identity to be able to publish
pull-request reviews and commit statuses on the project's repository, the
project identity to be able to read its pull requests and commit statuses, and
a published procedure that answers `runner.review_needed` for the project.
Each is checked where the setting is admitted and by the installation check,
under `project/instrukt/policy/checks-and-guards`; a missing one is refused
there with its repair, and nothing running handles it. What is verified about
the identities is work's (`project/instrukt/spec/feature/work/github-apps`).

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
  another head replaced before its request reached a reviewer and that did not
  become current again.
- Each reviewed pull request keeps its own reviewer thread, which automation
  never settles (`project/instrukt/spec/feature/runner/procedure-standing-agent`).
- A reviewer that keeps ending its turn without a verdict is asked again at
  growing intervals for as long as the head stays current. The head carries no
  review status meanwhile and, where the status is required, cannot merge.
- One computer observes a project's pull requests at a time. Another computer
  recognises a verdict only once it is published on GitHub; a review in flight
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
