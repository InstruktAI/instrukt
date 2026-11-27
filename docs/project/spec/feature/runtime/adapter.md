---
description: Runtime-neutral adapter facade — universal live spine for unattended start, send, live state and turn observation, execution roots, listing, and caller identity, plus declared optional progressive capabilities with zero live runtime-database reads.
visibility: public
delivered_by:
  - thin-live-runtime-boundary
---
# Runtime Adapter — Spec

## What it is

The `runtime` bounded context owns the runtime-neutral adapter facade, neutral
session and capability vocabulary, and caller-identity recognition. Consuming
bounded contexts — `runner`, `company`, `publication`, `configuration`, `work`,
and `workflow` — depend solely on this neutral facade; concrete runtime
adapters live in separate adapter modules wired only at the CLI composition
root so adding another runtime is purely additive and leaves every generic
specification unchanged.

The runtime owns session lifecycle, input delivery, restart, revival, handover,
live transcripts, and cleanup. instrukt owns its work facts, durable bindings,
events, schedules, active-speaker tracking, and delivery bookkeeping.

## Canonical fields

### Universal live spine

Every compiled runtime adapter implements this minimal live boundary using
structured inputs and structured results. A session reference is
`<runtime>:<native-id>`; instrukt invents no replacement session identity.

| Primitive | Contract |
| --- | --- |
| `start` / `start_or_wake` | Takes an execution checkout, durable semantic key, frozen display title, opening brief, optional workspace placement, and assignment facts; returns the existing or newly started session reference and runtime acceptance, separately from domain completion. |
| `send` | Takes a native session reference, structured input, delivery intent (`as soon as possible` or `only at entry`), and optional idempotency command key; returns runtime acceptance separately from entry consumption and turn completion. |
| `read_state` | Takes a scoped native session reference; returns neutral operational state, presence, native project identity, active/latest turn correlation when available, settled metadata, workspace metadata, and native activity timestamp with provenance. |
| `caller_identity` | Takes the selected adapter and caller environment; returns the native session reference and any verified agent identity, or no caller. Performs no runtime I/O. |
| `execution_root` | Takes an execution root path and a mode (`check` or `ensure`); reports whether the selected runtime holds a live project rooted exactly at that root, creating one in `ensure` mode, together with the runtime's own door for adding one. Never selects, alters, or removes an ancestor or descendant project; an unreachable or refusing runtime returns its cause and no answer. |
| `list` | Takes a registered project checkout; returns operator-facing session summaries with native identity, display title, neutral state, and available native activity timestamps with provenance. |

### Neutral state, presence, delivery, and activity vocabulary

- **Operational state:** `running`, `needs-input`, `input-ready`, `failed`, and
  `gone` remain distinct. An idle or exited process supplies no work outcome;
  an unrecognised response shape, command failure, or unreachable runtime is an
  observation error, never `gone` or success.
- **Presence:** `present`, `archived`, and `absent` distinguish a live or
  settled session from a confirmed archived session and an explicit
  unknown-session response.
- **Delivery realization:** An adapter declares whether `as soon as possible`
  delivery realizes as steering, queue-only, or entry-only. Runtime acceptance
  is transport evidence, distinct from entry consumption and domain completion.
- **Activity timestamps:** Filtering timestamps carry adapter-declared meaning
  and provenance; an unknown timestamp stays explicitly unknown and never
  substitutes local observation time.
- **Native project identity:** Optional metadata distinct from instrukt project
  identity and the scoped session reference.

### Zero live runtime-database reads

All live operational paths — work dispatch, standing-agent activation and input
observation, company conversation delivery and active-speaker verification, MCP
access, admin-DM configuration patching, knowledge-proposal admission, review
admission, and operational session listing — operate exclusively over the live
adapter spine and instrukt-owned durable state. No live operational path opens
or reads a runtime's internal database.

Any read-only access to a runtime's internal storage is strictly confined to
declared optional non-live capabilities (cold text source and periodic thread
retention) inside a concrete runtime adapter, never invoked on live
operational paths.

### Declared optional progressive capabilities

Every capability beyond the universal live spine is explicitly declared by an
adapter. An unsupported capability is distinct from a transient runtime
failure: an unsupported operation reports the runtime and missing capability,
while an unavailable runtime leaves work pending for retry. Each consuming
bounded context declares in its own specifications which neutral capabilities
its features require.

| Capability | Neutral contract |
| --- | --- |
| Idempotent start and send keys | Accepts caller-captured command and session identities before creation or dispatch (`dispatch_send` / keyed launch) and exposes live reconciliation of accepted, rejected, or absent attempts without creating a second session or turn. |
| Input consumption and assignment observation | Reports through live runtime APIs whether a delivered input entered an active run and exposes bounded initialization/waiting evidence without reading a runtime's internal database. |
| Quota-aware model selection and readiness | Translates neutral profile, model, effort, and extra options, validates options against runtime-reported model descriptors where available, supplies selection-realization diagnostics, and returns provider readiness and usage-window facts from a background refresh. Selection order, quota marking, expiry, and fallback policy belong to the consumer (`project/instrukt/spec/feature/runner/session-continuity`, `project/instrukt/spec/feature/configuration/agent-selection`). |
| Configured agent runtimes | Reports enabled agent harness instances, their wire-to-canonical driver identities, and their resolved instruction home directories so publication can write global agent files; an unreachable runtime answers nothing (`project/instrukt/spec/feature/publication/agent-entrypoints`). |
| Workspace isolation (`create_worktree`, `attach_worktree`) | Creates a managed worktree on a specified remote base and branch, attaches a successor to a verified existing worktree, and exposes typed native delegation capability metadata (`project/instrukt/spec/feature/workflow/workflow-definition`). |
| Cold text source | Optional non-live read-only text source over retained conversations and finished messages (excluding runtime background-command reports and captured context payloads), returning opaque positions for incremental extraction (`project/instrukt/spec/feature/runner/history`). |
| Thread retention / archiving | Optional non-live inspection of finished threads (`finished_threads`: finishing time, latest activity, quietness, and pin state) and guarded archiving (`archive`: rechecking that the observed finished, quiet, unpinned state still holds before mutating, returning archived, already archived, or changed) (`project/instrukt/spec/feature/runner/thread-retention`). |
| Multi-account provider isolation | Optional ability to host multiple isolated model-provider accounts on one computer so Tier-1 shared-host companies run on their own subscriptions (`project/instrukt/spec/feature/company/company-containers`). |
| Remote peer-linking | Optional ability for an operator's host runtime to link into an isolated Tier-2 company container's runtime over a private authenticated network boundary, and optionally allow a technical company owner to link their own client account scoped strictly to that container (`project/instrukt/spec/feature/company/company-containers`). |
| Notification (`notify`) | Optional runtime-native notification accepting a session or work-item reference and deep link (`project/instrukt/spec/feature/runner/anomaly-notification`). |
| Conversation link (`link`) | Optional sharing of human clarifications between linked conversations, identified as the human's words. |

## Use cases

This specification defines the lower-level runtime-neutral adapter contract and
capability catalog consumed by higher-level bounded contexts; user-facing goals
that exercise these operations live in the consuming feature specifications
(`project/instrukt/spec/feature/runner/session-continuity`,
`project/instrukt/spec/feature/company/conversations`,
`project/instrukt/spec/feature/company/company-containers`,
`project/instrukt/spec/feature/runner/history`,
`project/instrukt/spec/feature/runner/thread-retention`,
`project/instrukt/spec/feature/runner/requested-review`, and
`project/instrukt/spec/feature/publication/agent-entrypoints`).

## Known caveats

- Concrete transport details, vendor command grammars, and provider-specific
  option mappings belong exclusively to each runtime's own adapter
  specification.
- How global `runner.runtime` is configured and validated belongs to
  `project/instrukt/spec/feature/configuration/layered-configuration`.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
- [project/instrukt/spec/feature/company/conversations](../company/conversations.md)
- [project/instrukt/spec/feature/company/company-containers](../company/company-containers.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](../runner/procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/history](../runner/history.md)
- [project/instrukt/spec/feature/runner/thread-retention](../runner/thread-retention.md)
- [project/instrukt/spec/feature/runner/requested-review](../runner/requested-review.md)
- [project/instrukt/spec/feature/configuration/agent-selection](../configuration/agent-selection.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
