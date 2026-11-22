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

### Which heads are reviewed

| Pull request state | Outcome |
| --- | --- |
| Open, not draft, head branch in the project's repository, trusted author, current head carries no verdict from the configured reviewer identity | One review activation for that exact head. |
| Current head already carries the reviewer identity's verdict | None. |
| Draft | None until it is marked ready. |
| Owned by a todo's work lifecycle | None; that lifecycle reviews it. |
| Head branch in a fork | None. |
| Author outside the project's trusted authors | None. |
| Opened by an automated dependency-update bot | None; a failed dependency update is assessed under `project/instrukt/spec/feature/workflow/integration-gate`. |

A trusted author is the repository's owner, a member of its organization, a
collaborator, or the bot of one of the project's configured GitHub Apps,
recognised by its verified bot identity under
`project/instrukt/spec/feature/work/github-apps`. A head is reviewed once, however
often it is observed. A new push is a new head and is reviewed in its own right;
an earlier head's verdict never stands in for it. The activation starts an
independent reviewer on that exact head under
`sdlc/procedure/lifecycle/work/review-commits`, with no involvement from the
authoring session.

A change-requesting review reaches an authoring thread that watches the pull
request through its runtime's pull-request watch; the fix it pushes is a new
head and is reviewed automatically.

### Enablement and prerequisites

The capability is enabled per project. For a project that has not opted in it
observes no pull requests, starts no reviews and publishes nothing; reviews the
project already receives are unaffected. Opting in requires the configured
reviewer identity to be able to publish pull-request reviews and commit statuses
on the project's repository. That ability is checked where the setting is admitted and by the installation
check, under `project/instrukt/policy/checks-and-guards`; a missing permission
is refused there with its repair, and nothing running handles it.

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
- Every new head of every eligible pull request starts a review.
- A pull request outside eligibility receives no review status. A canonical
  branch that requires the status does not admit it.

## See Also

- [project/instrukt/spec/feature/runner/event-activation](event-activation.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../workflow/integration-gate.md)
- [project/instrukt/policy/checks-and-guards](../../../policy/checks-and-guards.md)
