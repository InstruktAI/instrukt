---
description: The standing integrator verifies an exact candidate before advancing the canonical branch and restores a failing canonical branch before repairing its cause.
delivered_by: [instrukt-first-build]
---
# Integration Gate — Spec

## What it is

A developer's finalized work reaches the canonical branch through a verified
integration candidate. The standing integrator owns candidate composition,
failure interpretation, promotion and recovery. GitHub supplies CI results
and branch protection; the integrator supplies the decisions.

## Canonical fields

### Candidate and verification

The integrator merges finalized candidates into its own integration worktree
and pushes the resulting commit to `integrate`. It may batch
candidates according to its judgment. A merge conflict remains work for the
integrator to resolve before there is a candidate to verify.

Each tick compares local main with remote main using local git, without an
API call. When local main is ahead and no candidate is pending, the runner
emits `runner.main_ahead` to wake the integrator. The integrator fetches the
review notes ref and reads direct commits' git-note verdicts
(`local/projects/instrukt/spec/feature/repository/attributed-commit`) and takes settled direct commits through the same
candidate and verification gate. It takes commits noted `approve` or `exempt`,
stopping at the first commit with neither verdict.

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

For a project without CI the integrator runs the project's configured test
action in the integration worktree against the candidate. Success supplies
the local admission evidence for that exact commit; failure remains with the
integrator for judgment under the same remediation routes. An unavailable or
failing action does not establish a green candidate.

This local action is the integration gate. Builders, reviewers and the pure
workflow evaluator do not add suite execution to their steps. Project-owned
hooks and CI retain their own checks.

### Recovery of a red canonical branch

If the canonical branch becomes red despite admission, the integrator reverts
the offending delivery first. The revert passes through `integrate` like
any change to main; its CI is the fast path back. It restores the usable shared baseline before
repairing the cause in a separate todo, and keeps subsequent deliveries from
building on the known-red baseline.

When the integrator cannot restore the baseline, the condition is an anomaly
that can require notification. Ordinary approval and new-intent requests
remain in the responsible agent's thread. The runner owns whether and how an
anomaly is notified; the integration gate reports the unresolved failure.

### Use cases

#### UC-IG1: Admit only a verified integration candidate

```gherkin
Scenario Outline: Promote a verified candidate
  Given a developer has finalized work ready for integration
  And the project uses <verification> to verify integration candidates
  When the developer has that work integrated
  Then the canonical branch advances only to the exact candidate that passed verification
  And a failing or unfinished candidate remains unpromoted

  Examples:
    | verification                   |
    | required CI checks             |
    | a configured local test action |
```

#### UC-IG2: Restore a usable shared baseline

```gherkin
Given a developer's project has a failing canonical branch after a delivery
When the developer has the failed delivery recovered
Then the offending delivery is reverted before its cause is repaired
And repair proceeds as separate work from restoration of the shared baseline
```

## See Also

- `general/principle/trust-and-guardrails` — admission protects the shared work object.
- `general/principle/intelligence-boundaries` — the integrator interprets failures and selects remediation.
