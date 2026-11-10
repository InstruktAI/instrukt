---
description: The standing integrator verifies an exact candidate before advancing the canonical branch and restores a failing canonical branch before repairing its cause.
binding_reservations:
  UC-IG6: instrukt-first-build
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

The finalized candidate is the shared todo branch head after successful
finalizer completion and release, current independent approval and readiness under
`project/instrukt/spec/feature/workflow/delivery-cycle`. The integrator reads
that branch and stamped commit; it does not inherit a shared todo worktree.

The integrator merges finalized candidates into its own integration worktree
and pushes the resulting commit to `integrate`. It may batch
candidates according to its judgment. A merge conflict remains work for the
integrator to resolve before there is a candidate to verify.

The integrator receives reviewed todo or one-off worktree heads. Main has no independent authoring commits to discover: only the integrator merges candidates, and publication goes live atomically at that integration. Reviewed preparation docs arrive through `workflow.preparation_reviewed` and are integrated as soon as possible before any build. Their clean `<slug>/prepare` worktree and local/remote branch are removed after confirmed integration; work starts separately from the new main. A dependent todo waits for delivery of the contracts it requires.

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

| Observed result | Integrator obligation |
| --- | --- |
| Required CI checks are green for the candidate | Fast-forward the canonical branch to that exact commit and push it. |
| Candidate CI is red | Read the failure and choose the remediation: return it to the responsible todo as findings, make a trivial fix, or isolate the offending member of a batch. |
| Verification has not completed or its result belongs to another commit | Keep the candidate unpromoted. |
| The canonical branch no longer admits the verified fast-forward | Reconcile a candidate against the current canonical branch and obtain verification for the resulting commit. |

Any candidate altered by remediation is verified in its resulting form before
promotion. The standing integrator drives this process; an automatic merge
queue does not select, merge or discard candidates on its behalf.

### Canonical-branch guard

For a project with CI, GitHub branch protection requires the applicable green
status checks on the candidate commit before a push can advance the canonical
branch. A missing or failing required result supplies no admission. This is
an integrity guard in addition to the integrator's judgment.

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

### UC-IG5: Build after preparation contracts are published

```gherkin
Given a developer's todo has documentation committed in its preparation worktree
When independent review approves that exact preparation head
Then an occurrence event wakes the integrator to integrate it through the candidate gate as soon as possible
And its documentation is published before the implementation starts
And the integrator removes the merged preparation worktree and its local and origin branch
And work starts separately from freshly fetched remote main
```

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
