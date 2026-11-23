---
description: The standing integrator verifies an exact candidate before advancing the canonical branch and restores a failing canonical branch before repairing its cause.
---
# Integration Gate — Spec

## What it is

A developer's finalized work reaches the canonical branch through a verified
integration candidate. The standing integrator owns candidate composition,
failure interpretation, promotion and recovery. GitHub supplies CI results
and branch protection; the integrator supplies the decisions. instrukt
itself never moves the canonical branch: it wakes the integrator with each
exact candidate and that candidate's own result, and keeps review verdicts
readable. Promotion, failure judgment, revert-first recovery and separate
repair are the integrator's obligations under its procedure, guarded by
branch protection.

## Canonical fields

### Candidate and verification

The finalized candidate is the shared `<slug>/work` branch head carrying an
independent reviewer's `build_review` `approve` stamp bound to that exact commit
SHA and `--finalize-ready` under
`project/instrukt/spec/feature/workflow/delivery-cycle`. The integrator reads
that branch and stamped commit; it does not inherit a shared todo worktree.

The integrator merges finalized candidates into its own integration worktree
and pushes the resulting commit to `integrate`. It may batch
candidates according to its judgment. A merge conflict remains work for the
integrator to resolve before there is a candidate to verify.

The integrator receives reviewed todo or one-off worktree heads. Main has no
independent authoring commits to discover: only the integrator merges
candidates, and publication goes live atomically at that integration. A
dependent todo waits for delivery of the contracts it requires.

The active candidate is the fetched `origin/integrate` head while it is not
already contained in remote canonical main. An absent integrate ref or a head
already contained in remote main means no candidate is pending. A failed or
unverified candidate remains active until the integrator promotes it or
replaces it with a repaired candidate. Candidate observation uses these refs,
not a separate candidate registry.

CI runs on that candidate push. Each runner tick checks the active integrate
candidate’s GitHub CI status with a conditional request, respecting the
source’s `X-Poll-Interval` header, and emits
`runner.ci_completed`. That event wakes the standing integrator and identifies
the candidate commit and check outcome. Promotion applies to
exactly the verified commit; a different commit does not inherit that result.

A candidate's CI result is read from the commit's combined status and its
check runs:

- **Green:** the combined status is `success`, or there are no statuses, and
  every check run has completed as `success`, `neutral` or `skipped`; at least
  one status or check run exists.
- **Red:** any status or check run has failed.
- **Pending:** anything is still running, or nothing has reported yet.
- **`no_ci`:** nothing has reported for the candidate and the canonical
  head carries no status and no check run; the project has no CI.

<!-- planned:event-driven-pr-review-gate -->
On a project that opted in to automatic review, the reviewer bot's review
status is not a CI result (`project/instrukt/spec/feature/work/github-apps`).
Classification leaves it out, and a project whose commits carry nothing else
has no CI. There a
candidate's result is complete only once its review is, read from work's
answer on the candidate commit
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence) and from the review status the commit carries. The first row that
applies decides.

| Work's answer on the candidate commit, and its review status | The candidate's result |
| --- | --- |
| The commit's current verdict with work is needs work | `failure`, whatever review status the commit still carries. |
| The commit carries no review status | Pending. |
| The review status is a failure | `failure`. |
| Anything else | The CI result. |

A status the reviewer bot published for an earlier version of the verdict
therefore completes no result that the current verdict withholds. From the
tick that finds needs work current, the result is a failure, before GitHub
shows the failure status.
A result that was reported complete before the verdict changed is not
recalled. On a
project that has not opted in,
classification reads every status and check run as before, a review status
published earlier included.
<!-- /planned:event-driven-pr-review-gate -->

| Observed result | Integrator obligation |
| --- | --- |
| Required CI checks are green for the candidate | Fast-forward the canonical branch to that exact commit and push it. |
| Candidate CI is red | Read the failure and choose the remediation: return it to the responsible todo as findings, make a trivial fix, or isolate the offending member of a batch. |
| Verification has not completed or its result belongs to another commit | Keep the candidate unpromoted. |
| The canonical branch no longer admits the verified fast-forward | Reconcile a candidate against the current canonical branch and obtain verification for the resulting commit. |

Any candidate altered by remediation is verified in its resulting form before
promotion. The standing integrator drives this process; an automatic merge
queue does not select, merge or discard candidates on its behalf.

<!-- planned:event-driven-pr-review-gate -->
### Review coverage of a candidate

On a project that opted in to automatic review, each tick that observes an
active candidate with no verdict of its own and no review request selected
or handed over derives whether the content the candidate adds to the canonical branch is
covered. A candidate whose derived approval GitHub does not show yet as its
review status is derived again in the same way on each tick, before anything
of that approval is published. A verdict the candidate commit already holds, as when it was reviewed
as a pull request's head, is its verdict here as well when work recorded it
as covering the canonical branch
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence, Range). Coverage applies the
review-currency derivation of
`project/instrukt/spec/feature/workflow/artifact-currency` to the candidate: it
follows the candidate's first-parent commits back to the canonical branch, and
each of them must be one of these.

| Commit on the candidate's first-parent line | Covered when | Evidence |
| --- | --- | --- |
| An approved head | It is exactly a head that holds a current independent approval of everything it adds to the canonical branch. It covers every commit it contains. | An independent approval bound to that exact commit in a todo's record: a finalized commit's build-review approval (`project/instrukt/spec/feature/workflow/delivery-cycle`), or the approval of a reviewed preparation head with its report, which the record keeps readable (`project/instrukt/spec/feature/work/stamps`, Historical preparation-head stamps). For any other head, work's current evidence for that commit is an approving head verdict that covers the canonical branch (`project/instrukt/spec/feature/work/github-apps`). |
| A merge of an approved head | Its second parent is exactly such a head, and its tree is what git's own merge of its two parents produces. A commit that an approved head merely contains, and a head that contains an approved commit, are not approved heads. | The head's approval as above; the merge reproduced from its parents. |
| A merge of the canonical branch | Its second parent is contained in the canonical branch, and its tree is what git's own merge of its two parents produces. | Reproduced from its parents. |
| A documentation fold | Its tree is what folding one delivery's pending documentation produces from its parent's tree, and that delivery's approved head is incorporated below it in this candidate. | The fold reproduced for that delivery (`project/instrukt/spec/feature/docs/pending-documentation`, Delivery fold). |
| A revert of the failing delivery | The canonical head the candidate is based on is one the runner observed red, and the commit undoes only what that head's own promotion added: its tree is what git's own revert of one commit that promotion added produces from its parent's tree, or it is exactly the tree the canonical branch had before that promotion. | The runner's red observation of that head; work's evidence of that head as a candidate, which names the canonical head it was promoted from; the revert reproduced. |
| A commit its author exempted | Work holds an exemption for it. | Work's exemption record (`project/instrukt/spec/feature/work/github-apps`). |

Approval of a head that is not a todo's is work's answer on that commit's
current evidence and nothing else
(`project/instrukt/spec/feature/work/github-apps`, Review evidence). The review
status a commit carries on GitHub is no input to coverage, and neither is a
verdict work only recognised as published. When the head was reviewed decides
nothing: a review that finished before its head was incorporated, or before
the project opted in, covers the head once work holds its verdict. The approval
is of that exact commit, and anything the candidate adds beyond it is judged by
its own row.

An approval in a todo's record covers the exact commit it names and no other:
a head that changed after its approval, and one whose approval a later verdict
replaced, is not an approved head.

The commits a red head's promotion added are the first-parent commits after
the canonical head it was promoted from, up to that head. Where work holds no
evidence of the red head as a candidate, that extent is not known and no
commit is covered as a revert. A revert that reaches further back than that
promotion, or a tree some earlier point of the canonical branch once had,
removes deliveries the red observation says nothing about and is not covered
this way.

| Finding | What follows |
| --- | --- |
| Every commit is covered | Work records a derived approval with the canonical head and every covering item, each named with the kind of evidence it rests on and where that evidence is kept, and publishes it (`project/instrukt/spec/feature/work/github-apps`). A derived approval recorded earlier that GitHub does not show yet is published with this finding's coverage. The candidate owes no review, and a request for it that is still queued ends. |
| A commit is not covered | The candidate owes a review. A derived approval recorded earlier that GitHub does not show yet is withdrawn through work and publishes nothing. The runner asks for an independent review of the exact candidate (`project/instrukt/spec/feature/runner/pull-request-review`, Candidate requests), and that review's verdict is the candidate's review status. |
| The repository cannot answer | Nothing is recorded, withdrawn, published or asked; the next tick derives again. |

A derived approval thus reaches GitHub only from a finding of the tick that
publishes it: a covering head whose approval a needs-work version replaced
gives the candidate no first approving status.
Once GitHub shows the status, a later version of a covering head withdraws
nothing.

A candidate is never approved on its covered part alone, and a different
candidate commit is derived afresh. Whether a commit owes review stays its
author's decision under `general/policy/commits`: a reviewed head and an
exemption are that decision on record. A commit that carries neither is asked
for, so nothing reaches the canonical branch because its author forgot to ask.

Reproduction compares trees and judges no content
(`general/principle/intelligence-boundaries`): a commit either equals what
instrukt computes from its parents or it is not covered by reproduction.
<!-- /planned:event-driven-pr-review-gate -->

### Canonical-branch guard

For a project with CI, GitHub branch protection requires the applicable green
status checks on the candidate commit before a push can advance the canonical
branch. A missing or failing required result supplies no admission. This is
an integrity guard in addition to the integrator's judgment.

<!-- planned:event-driven-pr-review-gate -->
Where the repository owner also requires the review status from the reviewer
bot, the same push needs that status on the candidate commit. Requiring it is
the owner's administration act
(`project/instrukt/spec/feature/runner/pull-request-review`).

GitHub checks the status the commit carries, which is the last verdict the
reviewer bot published. After a needs-work version was admitted and until its
failure status is published, an approving status published before still
satisfies this guard, and GitHub would accept the push. What holds the
candidate back in that interval is its result, which is a failure once needs
work is current (Candidate and verification); the status changes only when
the current verdict is published
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence).
<!-- /planned:event-driven-pr-review-gate -->

The canonical-branch push promotes the exact candidate without creating an
untested merge commit. That push is also the publication boundary observed by
the enabled code-graph capability; this context neither generates a graph nor
uses graph freshness as an integration check.

### Project without CI

Whether a project has CI is decided from the canonical branch's head commit,
never from a candidate: a canonical head carrying no status and no check run
means the project has no CI. A just-pushed candidate with nothing reported yet
is pending, not evidence of a project without CI.

For a project without CI the integrator runs the project's documented test
command in the integration worktree against the candidate. Success supplies
the local admission evidence for that exact commit; failure remains with the
integrator for judgment under the same remediation routes. An unavailable or
failing command does not establish a green candidate.

This local check is the integration gate. Builders, reviewers and the pure
workflow evaluator do not add suite execution to their steps. Project-owned
hooks and CI retain their own checks.

### Recovery of a red canonical branch

If the canonical branch becomes red despite admission, the integrator reverts
the offending delivery first. The revert passes through `integrate` like
any change to main; its CI is the fast path back. It restores the usable shared baseline before
repairing the cause in a separate todo, and keeps subsequent deliveries from
building on the known-red baseline. The integrator judges whether the failure
warrants a bug, deduplicates against existing work and files it once when
warranted; the runner never turns every red check into a bug automatically.

<!-- planned:event-driven-pr-review-gate -->
On a project that requires the review status, a candidate that reverts the
failing delivery of a red canonical head is covered by reproduction and carries
the status without waiting for a review (Review coverage of a candidate). Which
delivery to revert remains the integrator's judgment; a recovery that removes
more than the red head's own promotion added is reviewed as an exact candidate.
<!-- /planned:event-driven-pr-review-gate -->

When the integrator cannot restore the baseline, the condition is an anomaly
that can require notification. Ordinary approval and new-intent requests
remain in the responsible agent's thread. The runner owns whether and how an
anomaly is notified; the integration gate reports the unresolved failure.

### Failed dependency updates

`runner.dependency_failing` reaches the standing integrator through the same
failed-check delivery path as `runner.main_failing`, with the exact dependency
branch, head and failed check run. Observation is owned by
`project/instrukt/spec/feature/runner/event-activation`; this context owns the
response. The integrator reads the check evidence, judges whether it warrants
a bug, deduplicates against existing work and files it once when warranted.
Dependency-branch failure does not imply red main or trigger a revert of main.

### Mechanical canonical-checkout synchronization

On each tick, before dispatch, the runner observes remote main with Git and
fetches the observed commits without a GitHub API call. When remote main is
ahead, it fast-forwards the clean project checkout's local main to that exact
commit with `git merge --ff-only --no-overwrite-ignore`. The existing post-merge
hook publishes documentation and hands the revision to blueprint when enabled.
This runs for considered projects with todos or events enabled, independently
of the events switch. Equal heads require no merge. Dirty checkouts, a checkout
on another branch, failed fetches and non-fast-forward history are reported;
no checkout is forced, switched, stashed or discarded. A later tick retries.
The integrator promotes remote main; it no longer synchronizes other checkouts.

`runner.main_failing` wakes the integrator for revert-first recovery. The
existing recovery obligation keeps subsequent deliveries from building on the
known-red baseline; work-dispatch defines no outstanding-event admission check
or automatic hold. Mechanical synchronization and new phase-base fetching add
no red-main hold and do not interpret the event as a second gate.

## Use cases

### UC-IG1: A developer's integrator learns each candidate's own verification result

```gherkin
Given a developer has finalized work ready for integration
When the work is finalized
Then the integrator is woken to integrate it
When its candidate's CI completes
Then the integrator is woken with that exact candidate and its result
And while the candidate's CI is still running, nobody is woken for it
When a red candidate is replaced by a repaired one
Then the integrator is woken with the repaired candidate's own result
But when the project has no CI
Then the integrator is woken with the exact candidate and told the project has no CI
```

### UC-IG2: A revert is verified as its own candidate

```gherkin
Given a developer's project verifies its canonical branch with CI
When the CI of the canonical branch's latest commit reports a failure
Then the integrator is woken with that exact commit and its failure
When a revert of the failing delivery is pushed for integration
Then the integrator is woken with the revert's own verification result
And the failing result of the reverted delivery is never reported for the revert
```

### UC-IG3: Discover direct-main commits (retired)

Retired: main has a single writer and no direct-main authoring pickup.

### UC-IG4: Read contracts another computer has integrated

```gherkin
Given another computer's integrator has pushed reviewed and verified documentation to canonical main
And this computer's published project checkout still has the previous main
When this computer's runner ticks
Then it fetches and fast-forwards the clean canonical checkout
And the post-merge hook publishes the new documentation for this computer's agents
But a dirty checkout or non-fast-forward history is reported and preserved for retry
```

### UC-IG5: Build after preparation contracts are published (retired)

Retired: preparation and build share one `<slug>/work` worktree under the
unified `todo-orchestrator`, with no separate `<slug>/prepare` integration
handoff before build.

### UC-IG6: A developer gets failed dependency updates assessed without watching every bot PR

```gherkin
Given a developer has dependency-update failure evidence for their integrator to assess
When their integrator assesses that evidence
Then it judges whether the failure warrants a bug
And when warranted it records the bug work once, reusing matching existing work
```

## Known caveats

- A candidate that introduces CI into a project whose canonical head has none
  is judged `no_ci` when a tick reads it before its CI reports, and is then
  verified locally. Once its CI reports on the canonical branch, a red result
  is reported as a failing canonical branch (`runner.main_failing`), and CI
  governs from then on.

## See Also

- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md) — admission protects the shared work object.
- [general/principle/intelligence-boundaries](../../../../general/principle/intelligence-boundaries.md) — the integrator interprets failures and selects remediation.
- [project/instrukt/spec/domain/workflow](../../domain/workflow.md)
