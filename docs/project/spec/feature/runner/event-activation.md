---
description: Runtime-independent procedure subscriptions, external event sources and the fork-upstream response example.
visibility: public
delivered_by: [instrukt-first-build]
---
# Event Activation — Spec

## What it is

The events module makes a project's standing agents responsive to changes,
mostly from outside instrukt. A developer adopting instrukt can learn what
changed upstream without a CI pipeline driving the response.

## Canonical fields

### Subscription contract

The owning routing contract is
`project/instrukt/spec/feature/runner/procedure-subscription-routing`. Docs validates
the declaration and publication supplies its resolved facts and reach through
the generated index. The matcher is one independent retained consumer, with
per-procedure acceptance keys rather than another group or SQLite scan for each
procedure. General/domain and project-local subscription reach follow that
contract. Pending declarations stay inactive; event-source data selects no route.


Procedure frontmatter carries subscriptions. They travel with the
documentation even when events are disabled. The runner reads an event, resolves matching published subscriptions and durably accepts keyed activations. The runtime adapter owns the subsequent start or wake.

`modules.events` is the sole events switch, available in both layers (project overrides global). `events.watch` is only the project watch list. Off leaves existing subscriptions inert.
Todo pickup settings do not govern these activations.

Procedures subscribe to instrukt's vocabulary. Event-source adapters translate
native deliveries; procedures do not subscribe directly to one runtime's native
vocabulary. The runtime adapter receives the resulting assignment; event-source
translation remains independent of session execution.

Event activation works with the procedure's declared role independently of
workflow role-list membership and todo enablement. Reactions are agent-based;
script-run event actions and event-bound maintenance are outside scope.
Path-based post-commit follow-ups use git hooks, including sync hooks.

### Sources and reactions

The source names and canonical envelopes follow the runner domain's explicit naming
map. Their canonical envelopes carry a stable occurrence ID, project
origin and typed subject; external source payloads remain open domain facts.

| Origin | Source | Runner-facing meaning |
| --- | --- | --- |
| Workflow | `workflow.preparation_reviewed` (occurrence) | Approved exact preparation head wakes the integrator for immediate documentation integration before build. |
| Repository maintenance | Observe remote main with Git on each tick, fetch and fast-forward a clean canonical checkout | Post-merge publishes docs and hands the revision to blueprint; dirty/diverged checkouts are reported and preserved, with no main-ahead event. |
| Workflow | `workflow.finalized` | Finalize-ready work wakes the subscribed standing integrator. |
| Repositories | `git ls-remote` for configured watched upstream refs on each tick | A changed upstream produces `runner.upstream_changed`. |
| Candidate CI | Conditional requests for the active integrate candidate's CI status per SHA, respecting the source poll interval | A completed result produces `runner.ci_completed` and wakes the integrator. When nothing has reported for the candidate and the canonical head carries no status and no check run, the result is `no_ci`. |
| Dependency-update CI | Remote branch heads whose names start with a configured project dependency-update prefix, using the same conditional CI requests | Each failed check run produces `runner.dependency_failing` with its exact branch, head, run identity, name and conclusion. It reaches the subscribed integrator through the same event log, transport and activation path as red-main checks. |
| Canonical CI | Conditional requests for the CI status of remote canonical main's head commit, respecting the source poll interval | When that commit's CI completes red, produce `runner.main_failing` once for that commit and wake the integrator. |

The active candidate SHA is the fetched `origin/integrate` head not yet
contained in remote canonical main, under the integration-gate contract.
An absent ref or an already promoted head means no candidate is pending.

The project layer selects watched refs with `events.watch: [{remote, ref}]`.
`ci.dependency_branch_prefixes` selects dependency-update branches. Only their
current remote heads are observed; an empty list disables this source. The
runner reports failed runs, including required runs, without deciding whether
their evidence warrants a bug. Integration owns that judgment under
`project/instrukt/spec/feature/workflow/integration-gate`. Repeated observation
of the same branch/head/run/conclusion is idempotent; a new head or failed rerun
is fresh evidence. The source needs the existing Git remote and GitHub observation
credentials, not a webhook or separate failure delivery service.

State watching needs no public address; signed webhook sources additionally use the opt-in ingress receiver. Each tick considers watched upstream refs, CI
status per active candidate SHA and the CI status of remote canonical main's
head commit, plus dependency-update branch check runs, subject to the source poll interval. It does not depend on delivery guarantees: a missed
tick is caught by the next tick reading current state.

A project's declared native or GitHub endpoint verifies and relays a signed
request as a cluster event. The request is an event fact in its own right;
repository polling remains an independent source-reconciliation mechanism.
External events carry the ingress computer as source. Only that computer
activates matching procedures, and a missing local home is a visible failure.

Polling checks only active subjects: a waiting integrate candidate, remote
canonical main's head commit, dependency-update heads, local items on this computer and configured
watched upstreams. All check-run pages are read before classification and failure delivery. Each
page participates in the existing conditional-request cache and source deferral.
API requests reuse
the previous ETag; an unchanged `304` does not consume GitHub rate quota.
Upstream refs use `git ls-remote`, consuming no API quota. Where GitHub sends
`X-Poll-Interval`, the runner never polls that source faster.

State watching is source acquisition. Standing agents still
wake on the resulting event; they do not repeatedly inspect repositories to
discover whether anything changed. Time-based task due logic belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Fork-upstream response

When a watched upstream ref changes, its subscribed standing agent is activated.

The shipped `general/procedure/upstream-watch` subscribes a standing agent to
`runner.upstream_changed` and owns assessment, admission, routing and reporting
of the upstream response, including preservation of local modifications for a
fork reconciliation.

Human questions stay in the responding agent's thread, as governed by
`project/instrukt/spec/feature/runner/anomaly-notification`.

### Delivery and restart boundary

An event names work to reconsider; the current work record remains the source
of workflow progress. A turn-completion event cannot stand in for a step's
artifact and stamp. Session observation follows
`project/instrukt/spec/feature/runner/session-continuity`.

Local event records survive restarts in A1's SQLite store. Publication to
scoped Redis is a separate effect, retried from the same local row with the same
occurrence ID and recorded publication coordinates. Redis groups retain
per-consumer/per-computer progress. Successful handling commits its local outcome,
handled event ID and settled local cursor before Redis acknowledgment. Duplicate
protection lasts seven days; retained stream replay is independently bounded.
Handlers reconcile their subject idempotently after interrupted acceptance.

Conditional-request validators are kept in the state database per project
and request, so a later invocation asks only for what changed, beside each
watched ref's last observed revision.

Delivery acceptance and runtime execution are separate completion boundaries.
The target contract retains procedure-first briefing, untrusted event provenance,
bounded batch selection and eligible delivery boundaries, with settings fixed
for a session including continuation after human settling of a standing thread. The owning contracts are
`project/instrukt/spec/feature/runner/procedure-standing-agent` and
`project/instrukt/spec/feature/runner/procedure-event-delivery`. Runtime realization
uses the selected adapter's capabilities;
routing itself starts no session. `run` hosts independent readers; `tick` drains
pending then new entries within a bounded invocation. Scheduled-only execution
reacts at the next wake while retained entries remain available.

A3's adapter owns per-key runtime discovery and ambiguous-start recovery under
`project/instrukt/spec/feature/runner/session-continuity`: a failed listing or
send leaves work pending and starts no replacement, and an uncertain start is
reconciled by key or explicit resolution. The delivery contract distinguishes runtime acceptance from entry consumption
and domain completion. ASAP uses steering when available and otherwise queued
input or idle entry; only-at-entry waits for idle. Missing steering is no finding.
Procedure-event-delivery owns selection, combining and untrusted data framing.

## Use cases

### UC-EA1

### UC-EA2: A developer's integrator learns of failed dependency checks

```gherkin
Given a developer has selected dependency-update branches for CI observation
And their integrator subscribes to dependency-check failure evidence
When a required check fails on a current selected branch head
Then their integrator receives that exact branch, commit and failed check result
And repeated observation of that same failure creates no duplicate delivery
But unrelated branches and successful or unfinished checks do not produce this failure delivery
```

## Known caveats

- Company/person origins and references travel intact, but resolving those homes
  is unsupported in this release and fails visibly. Unknown never means none.
- Runtime delivery equivalence remains subject to the S6/A3 feasibility gate. Adapter key-title discovery and ambiguous-start limits remain; a lost acknowledgment can repeat a send and requires owner idempotency.

## See Also

- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/event-transport](event-transport.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](procedure-standing-agent.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
