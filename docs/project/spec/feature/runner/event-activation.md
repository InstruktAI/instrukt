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

### Sources and reactions

The five source names remain unchanged as the runner domain's explicit naming
map records. Their canonical envelopes carry a stable occurrence ID, project
origin and typed subject; external source payloads remain open domain facts.

| Origin | Source | Runner-facing meaning |
| --- | --- | --- |
| Workflow | `workflow.finalized` | Finalize-ready work wakes the subscribed standing integrator. |
| Repositories | `git ls-remote` for configured watched upstream refs on each tick | A changed upstream produces `runner.upstream_changed`. |
| Local repository | Compare local main with remote main using local git on each tick; no API call | When local main is ahead and no candidate is pending, emit `runner.main_ahead` to wake the integrator. |
| Candidate CI | Conditional requests for the active integrate candidate's CI status per SHA, respecting the source poll interval | A completed result produces `runner.ci_completed` and wakes the integrator. When nothing has reported for the candidate and the canonical head carries no status and no check run, the result is `no_ci`. |
| Canonical CI | Conditional requests for the CI status of remote canonical main's head commit, respecting the source poll interval | When that commit's CI completes red, produce `runner.main_failing` once for that commit and wake the integrator. |

The active candidate SHA is the fetched `origin/integrate` head not yet
contained in remote canonical main, under the integration-gate contract.
An absent ref or an already promoted head means no candidate is pending.

The project layer selects watched refs with `events.watch: [{remote, ref}]`.
State watching needs no public address; signed webhook sources additionally use the opt-in ingress receiver. Each tick considers watched upstream refs, CI
status per active candidate SHA and the CI status of remote canonical main's
head commit, subject to the source poll interval. It does not depend on delivery guarantees: a missed
tick is caught by the next tick reading current state.

A project's declared native or GitHub endpoint verifies and relays a signed
request as a cluster event. The request is an event fact in its own right;
repository polling remains an independent source-reconciliation mechanism.
External events carry the ingress computer as source. Only that computer
activates matching procedures, and a missing local home is a visible failure.

Polling checks only active subjects: a waiting integrate candidate, remote
canonical main's head commit, local items on this computer and configured
watched upstreams. API requests reuse
the previous ETag; an unchanged `304` does not consume GitHub rate quota.
Upstream refs use `git ls-remote`, consuming no API quota. Where GitHub sends
`X-Poll-Interval`, the runner never polls that source faster.

State watching is source acquisition. Standing agents still
wake on the resulting event; they do not repeatedly inspect repositories to
discover whether anything changed. Time-based task due logic belongs to
`project/instrukt/spec/feature/runner/task-timing`.

### Fork-upstream response

The shipped `general/procedure/upstream-watch` subscribes a standing agent to
`runner.upstream_changed`. The
agent reports the upstream changes and offers reconciliation that retains the
fork's local modifications. A person's affirmative response turns that offer
into a todo with review through work and workflow. The
event itself does not authorize reconciliation of the fork.

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
sequential/batched selection and eligible delivery boundaries, with settings fixed
for a session including revival. The owning contracts are
`project/instrukt/spec/feature/runner/procedure-standing-agent` and
`project/instrukt/spec/feature/runner/procedure-event-delivery`. Runtime realization
remains an A3 integration feasibility gate;
routing itself starts no session. `run` hosts independent readers; `tick` drains
pending then new entries within a bounded invocation. Scheduled-only execution
reacts at the next wake while retained entries remain available.

A3's adapter owns per-key runtime discovery and ambiguous-start recovery under
`project/instrukt/spec/feature/runner/session-continuity`: a failed listing or
send leaves work pending and starts no replacement, and an uncertain start is
reconciled by key or explicit resolution. Those primitives do not establish
hook provenance or eligible delivery while a session is busy. The procedure-event handoff
preserves this target delivery contract rather than equating a plain send with it.

### Use cases

#### UC-EA1: Learn about an upstream change before reconciling a fork

```gherkin
  Given a developer has adopted instrukt for a fork with local modifications
  And the developer has enabled event responses for its upstream
  When the upstream changes
  Then a standing agent reports the changes
  And offers reconciliation that retains the fork's local modifications
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
