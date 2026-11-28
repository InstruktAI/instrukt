---
description: The runtime bounded context — thin vendor-neutral adapter boundary between instrukt and any agent runtime, progressive capability catalog, composition-root wiring, and concrete runtime adapters.
visibility: public
delivered_by: [thin-live-runtime-boundary]
---

# Runtime — Spec

## What it is

The runtime bounded context owns the vendor-neutral boundary between instrukt
and any concrete agent runtime (such as T3 Code). It defines the narrow
universal live spine every runtime adapter implements, the structured turn and
presence observations consumers rely on, the progressive optional capability
catalog through which richer runtime features are discovered and exercised,
and the concrete runtime adapter crates that translate those contracts onto
each runtime's supported public surfaces.

No live instrukt command, runner tick, or company conversation path reads a
runtime's private database. Every live operation crosses the runtime adapter's
authenticated public interfaces, and compile-time crate boundaries keep every
concrete runtime adapter behind the CLI composition root so no domain crate
imports a runtime implementation.

## Canonical fields

### Runtime adapter boundary

The runtime contract separates a required universal live spine from progressive
optional capabilities (`project/instrukt/spec/feature/runtime/adapter`):

- **Universal live spine:**
  - `start_session` — create a new runtime session with an idempotent client
    command identity, fixed session identity where supported, working
    directory, opening prompt, and optional role/model selection.
  - `send_input` — deliver an idempotent user input (`command_id` /
    `input_id`) to an existing session, returning accepted, replayed, or
    explicitly rejected status without guessing on ambiguous transport errors.
  - `read_state` — observe one session's semantic `Presence` (`Present`,
    `Archived`, `Absent`), lifecycle `State` (`InputReady`, `Running`,
    `NeedsInput`, `EndedFailed`, `Gone`, `Unknown`), structured turn
    correlation (`active_turn` and `latest_turn`), and pending input
    observations through authenticated live runtime interfaces.
  - `readiness` / `liveness` — report whether the selected runtime is reachable
    and ready for unattended operations.
  - `caller_identity` — environment-only recognition of the calling native
    session without runtime I/O, never selecting the runtime itself.
  - `configured_agent_runtimes` — report configured and enabled agent runtimes
    and their homes for publication's global agent files.
- **Structured turn correlation:**
  `Thread` never collapses an active run and a completed latest run into one
  ambiguous identifier. It exposes:
  - `active_turn: Option<ActiveTurn>` — populated only while a run is actively
    executing (`starting`, `running`, or `waiting`; never for `preparing`,
    `queued`, or terminal runs), carrying `run_id`, `input_id` (matching the
    dispatched `command_id` / native `userMessageId`), and opening-input
    inspection for envelope detection.
  - `latest_turn: Option<ObservedTurn>` — the most recent observed run with
    `run_id`, `input_id`, `state` (`Preparing`, `Queued`, `Active`,
    `Completed`, `Failed`), `started_at`, and `completed_at`.
  - Live observation queries (`accepted_start`, `observe_send`,
    `observe_input`, `assignment_evidence`, and held-session lookup) backed by
    authenticated live runtime projection/API reads rather than runtime
    database files.
- **Progressive optional capabilities:**
  Optional features are advertised by `Capability` and checked explicitly
  before use (`Worktree`, `NativeDelegation`, `AgentSelection`,
  `RuntimeProjectIsolation`, `ColdTextSource`, `Archive`). Adding a new
  runtime adapter requires only the universal live spine; missing optional
  capabilities degrade or refuse clearly at the feature boundary without
  weakening the spine.
- **Non-live isolated capabilities:**
  - `ColdTextSource` — cold, non-live extraction of historical conversation
    text and legacy record normalization for `history` only
    (`project/instrukt/spec/feature/runner/history`). No live runner, company,
    or review path consumes `ColdTextSource` or subscribes to runtime database
    change notifications.
  - `Archive` — listing quiet finished threads and archiving them for
    `thread-retention` (`project/instrukt/spec/feature/runner/thread-retention`).

### Runtime declarations and composition-root wiring

- **`RuntimeDeclaration`:** Data-only metadata published by each compiled
  runtime adapter (runtime name, advertised native and workflow capabilities,
  archive support, and shipped native-operation instruction catalog).
  `instrukt-cli` installs all compiled declarations into `instrukt-core` once
  at process startup (mirroring connector declarations), so `configuration`
  validates `runner.runtime`, retention eligibility, and instruction rendering
  without importing any concrete runtime adapter crate.
- **Adapter factory / registry:** `instrukt-cli` is the sole crate that imports
  concrete runtime adapter crates (such as `instrukt-runtime-t3`) and wires the
  selected `&dyn RuntimeAdapter` (and environment-only `CallerIdentityLookup`
  and `ConfiguredAgentRuntimes`) into `instrukt-runner`, `instrukt-company`,
  `instrukt-todos`, and `instrukt-core` publication hooks.

### Pinned T3 adapter contract

The T3 adapter (`crates/instrukt-runtime-t3`, pinned via
`crates/instrukt-runtime-t3/t3.version`) implements the universal live spine
over T3's authenticated live RPC and CLI/HTTP surfaces
(`project/instrukt/spec/feature/runtime/t3`):

- Live start uses `orchestration.launchThread` with deterministic `threadId`
  and `commandId` (matching `messageId`), so replay after an ambiguous reply is
  idempotent and never creates a duplicate thread or revives a deleted thread.
- Live input delivery uses `orchestration.dispatchCommand` with `messageId`
  equal to `command_id`; previously rejected commands surface T3's rejected
  command-receipt error so callers clear rejected sends rather than looping.
- Live state, active-turn correlation, `accepted_start`, `observe_send`,
  `observe_input`, and `assignment_evidence` read T3's authenticated
  `orchestration.getThreadProjection` RPC, which unconditionally retains all
  non-terminal (`preparing`, `queued`, `starting`, `running`, `waiting`) runs,
  their referenced user messages (`run.userMessageId`), and the latest run
  within its bounded projection window. Historical turns older than T3's
  bounded projection window remain outside live projection reads, with
  instrukt's `runtime_bindings` authoritative for identity.
- All reads of T3's internal `statev2.sqlite` are forbidden on live paths and
  remain isolated inside `instrukt-runtime-t3` exclusively for non-live
  `ColdTextSource` extraction and `Archive` finished-thread listing.
- Provider-free contract verification (`project/instrukt/spec/feature/runtime/t3-contract`)
  guards every T3 wire surface against the pinned release and keeps test
  doubles aligned with the real release.

### Domain events emitted

None. This context emits no domain events.

### Domain events consumed

None.

## Ubiquitous language

**Runtime adapter**
: The vendor-neutral Rust trait (`RuntimeAdapter`) in `instrukt-runtime`
  translating session lifecycle, live state observation, caller identity, and
  optional capabilities onto a concrete agent runtime.

**Universal live spine**
: The minimal set of live operations every runtime adapter implements:
  idempotent start, idempotent input delivery, live state and turn observation,
  readiness/liveness, environment-only caller identity, and configured agent
  runtime discovery.

**Active turn**
: A runtime run currently executing (`starting`, `running`, or `waiting`),
  identified by its `run_id` and opening `input_id` (matching the dispatched
  `command_id` / native `userMessageId`). Distinct from queued, preparing, or
  completed runs.

**Observed turn**
: The most recent run observed for a session (`latest_turn`), carrying its
  `run_id`, `input_id`, semantic `TurnState`, and start/completion timestamps.

**Input identity**
: The deterministic `command_id` / `input_id` recorded by instrukt before
  sending input and matched against the runtime's user message and run
  correlation fields to distinguish pre-start queueing, active execution, and
  post-turn completion without reading a runtime database.

**Capability**
: An optional runtime feature advertised by an adapter (`Worktree`,
  `NativeDelegation`, `AgentSelection`, `RuntimeProjectIsolation`,
  `ColdTextSource`, `Archive`) and checked explicitly by consumers.

**Runtime declaration**
: Data-only metadata (`RuntimeDeclaration`) installed once at CLI startup so
  `configuration` and `workflow` validate runtime selection and capabilities
  without depending on concrete adapter crates.

**Cold text source**
: The non-live optional capability (`ColdTextSource`) used exclusively by
  `history` to extract retained conversation text and normalize legacy records;
  never used on live runner, company, or review paths.

**Runtime anti-corruption layer**
: Each concrete adapter crate (such as `instrukt-runtime-t3`) translating
  vendor wire shapes, CLI commands, and non-live storage formats into
  `instrukt-runtime`'s neutral types.

## Ownership boundary

- **Owns:** the vendor-neutral `RuntimeAdapter` trait, neutral session/thread,
  turn-correlation (`ActiveTurn`, `ObservedTurn`, `TurnState`), presence,
  send/input observation, and error types; the progressive `Capability`
  catalog; `RuntimeDeclaration` metadata and composition-root registry
  contract; concrete runtime adapter crates (starting with
  `instrukt-runtime-t3`, its pinned release version `t3.version`, and its
  provider-free contract test suite); the zero-live-runtime-database-reads
  invariant; and non-live `ColdTextSource` and `Archive` adapter
  implementations.
- **Does not own:** keyed start orchestration, generation bookkeeping,
  `runtime_bindings` / `runtime_inputs` / `runtime_key_locks`, todo pickup,
  event routing, history indexing/search, or thread-retention policy (owned by
  `runner`, `project/instrukt/spec/domain/runner`); company conversation
  bindings, active-speaker policy, explicit replies, curated working status, or
  company containers (owned by `company`, `project/instrukt/spec/domain/company`);
  `instrukt.yaml` storage, validation, and interpolation (owned by
  `configuration`, `project/instrukt/spec/domain/configuration`); global agent
  file rendering (owned by `publication`, `project/instrukt/spec/domain/publication`);
  todo records and review evidence (owned by `work`, `project/instrukt/spec/domain/work`);
  or flavor manifests and workflow derivation (owned by `workflow`,
  `project/instrukt/spec/domain/workflow`).
- **Published contracts:**
  - `instrukt-runtime` neutral trait and types (`RuntimeAdapter`, `Thread`,
    `ActiveTurn`, `ObservedTurn`, `TurnState`, `Presence`, `Capability`,
    `ColdTextSource`, `ArchiveOutcome`, `Readiness`, `SessionsError`,
    `DispatchError`).
  - `RuntimeDeclaration` data installed at CLI startup for `configuration` and
    `workflow`.
  - Environment-only `CallerIdentityLookup` and `ConfiguredAgentRuntimes`
    implementations wired by `instrukt-cli` into `instrukt-core`.
  - Concrete adapter crate `instrukt-runtime-t3` wired exclusively by
    `instrukt-cli`.
- **Allowed dependencies:** `instrukt-runtime` depends only on `instrukt-core`
  (shared IDs, home paths, and configuration value types). Each concrete
  adapter crate (`instrukt-runtime-t3`) depends only on `instrukt-runtime`,
  `instrukt-core`, and `instrukt-outside`. No domain crate (`instrukt-core`,
  `instrukt-runner`, `instrukt-company`, `instrukt-todos`, `instrukt-blueprint`)
  ever depends on a concrete runtime adapter crate.
- **Implementation roots:** `crates/instrukt-runtime/` (neutral contract);
  `crates/instrukt-runtime-t3/` (T3 adapter, `t3.version`, and T3 contract
  wiring); `crates/instrukt-cli/` (sole composition root installing runtime
  declarations and wiring the concrete adapter).

## Event subjects and origins

This context emits no events.

## Use cases

- **Runtime adapter** — [project/instrukt/spec/feature/runtime/adapter](../feature/runtime/adapter.md)
  — vendor-neutral universal live spine, structured turn correlation, zero live
  runtime-DB reads, and progressive optional capabilities.
- **T3 runtime adapter** — [project/instrukt/spec/feature/runtime/t3](../feature/runtime/t3.md)
  — T3 Code implementation over authenticated RPC/CLI surfaces, live
  `orchestration.getThreadProjection` observation, and isolated non-live cold
  extraction and archiving.
- **T3 dependency compatibility** — [project/instrukt/spec/feature/runtime/t3-contract](../feature/runtime/t3-contract.md)
  — deterministic provider-free checks against the pinned T3 release,
  test-double agreement, and nightly drift detection.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — workspace crate DAG, CLI
  composition root, and runtime external-surface boundary.

## Context map position

- **Runtime → runner** — Customer / Supplier: supplies the universal live
  adapter spine, structured `Thread` / `ActiveTurn` / `ObservedTurn`
  observations, live start/send/input evidence, and optional capabilities
  (`Worktree`, `NativeDelegation`, `AgentSelection`, `RuntimeProjectIsolation`,
  `ColdTextSource` for history extraction, and `Archive` for thread retention).
- **Runtime → company** — Customer / Supplier: supplies live `read_state` with
  structured `ActiveTurn` / `ObservedTurn` correlation and opening-input
  inspection for fail-closed active-speaker resolution and live turn tracking,
  plus keyed session start and idempotent input delivery.
- **Runtime → publication** — Customer / Supplier: supplies configured agent
  runtimes and their homes from the selected adapter for rendering global agent
  files, through a core-declared interface wired by the CLI.
- **Runtime → configuration** — Published language: supplies compiled
  `RuntimeDeclaration` metadata (installed once at CLI startup) for validating
  `runner.runtime`, archive eligibility, and shipped runtime instruction
  catalogs, plus environment-only caller-identity lookup wired by the CLI.
- **Runtime → work** — Customer / Supplier: supplies runtime session references
  stored on todos and review evidence without translating native runtime
  mechanics.
- **Runtime → workflow** — Customer / Supplier: supplies the runtime capability
  vocabulary referenced in `workflow.yaml` `requires` and held-session
  observation for review coverage.
- **External agent runtimes** — Anti-corruption layer (ACL): each concrete
  adapter crate translates its runtime's RPC, CLI, HTTP, and non-live storage
  formats into `instrukt-runtime`'s neutral types.

## Known caveats

- `orchestration.getThreadProjection` in T3 bounds historical turns to a recent
  window (retaining all active/queued/preparing runs, their user messages, and
  the latest run unconditionally). Long-thread historical assignment lookups
  remain bounded by that window, with instrukt's `runtime_bindings`
  authoritative for identity.
- Provider-free T3 contract checks verify T3's protocol and projection shapes
  against the pinned release without live model-provider credentials; dynamic
  model-provider option descriptors remain checked offline against per-kind
  mappings (`project/instrukt/design/architecture/overview`).

## See Also

- [project/instrukt/spec/domain/context-map](context-map.md)
- [project/instrukt/design/architecture/overview](../../design/architecture/overview.md)
- [project/instrukt/spec/feature/runtime/adapter](../feature/runtime/adapter.md)
- [project/instrukt/spec/feature/runtime/t3](../feature/runtime/t3.md)
- [project/instrukt/spec/feature/runtime/t3-contract](../feature/runtime/t3-contract.md)
