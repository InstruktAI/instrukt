---
description: The runtime bounded context — thin vendor-neutral adapter boundary between instrukt and any agent runtime, progressive capability catalog, composition-root wiring, and concrete runtime adapters.
visibility: public
delivered_by: [thin-live-runtime-boundary]
---

# Runtime — Spec

## What it is

The runtime bounded context owns the vendor-neutral boundary between instrukt
and any concrete agent runtime. It defines the narrow universal live spine
every runtime adapter implements, the structured operational state and turn
correlation consumers rely on, the progressive optional capability catalog
through which richer runtime features are declared and exercised, and the
concrete runtime adapter crates that translate those contracts onto each
runtime's supported public surfaces.

No live instrukt command, runner tick, or company conversation path reads a
runtime's internal database. Every live operation crosses the runtime adapter's
authenticated public interfaces, and compile-time crate boundaries keep every
concrete runtime adapter behind the CLI composition root so no domain crate
imports a concrete runtime implementation.

## Canonical fields

### Runtime adapter boundary

The runtime contract separates a required universal live spine from declared
optional progressive capabilities (`project/instrukt/spec/feature/runtime/adapter`):

- **Universal live spine:**
  Every compiled runtime adapter realizes the six live spine operations
  (`project/instrukt/spec/feature/runtime/adapter`, Universal live spine):
  - `start` / `start_or_wake` — session creation, placement verification, and
    native project verification primitives (composed by `runner` for keyed
    start and present-idle rollover orchestration without upward crate
    dependencies).
  - `send` — delivering structured input to an existing session, returning
    runtime acceptance separately from entry consumption and turn completion.
  - `read_state` — observing one session's semantic presence (`present`,
    `archived`, `absent`), five operational states (`running`, `needs-input`,
    `input-ready`, `failed`, `gone`; unrecognised or unreachable responses
    return an observation error rather than a sixth state or `gone`),
    structured active and recent turn correlation, native project identity,
    settled metadata, workspace metadata, and native activity timestamp with
    provenance.
  - `caller_identity` — environment-only recognition of the calling native
    session without runtime I/O, never selecting the runtime itself.
  - `execution_root` — checking or ensuring a live runtime project rooted
    exactly at a given checkout path (never selecting or altering an ancestor
    or descendant project).
  - `list` — operator-facing session summaries for a registered checkout with
    native identity, display title, neutral state, and native activity
    timestamps with provenance.
- **Structured turn correlation:**
  Live session observations keep active execution distinct from queued or
  completed runs and carry bounded recent run history so callers never lose an
  overtaken turn across polls:
  - Active turn observation is structurally bound to present, active sessions
    (`running` or `needs-input`) and carries validated `RunId` and `InputId`
    identities plus a closed neutral opening-input value (raw text or
    explicitly unavailable; `runtime` performs no consumer envelope parsing).
  - Recent turn observations (`recent_turns`) carry each observed run's
    `RunId`, `InputId`, and payload-carrying `TurnState` (`Preparing`,
    `Queued`, `Active`, `Completed`, `Failed` with exact start/completion
    timestamp invariants).
  - Runtime errors use neutral typed `RuntimeError` variants (including
    explicit not-found and previously-rejected command outcomes), keeping
    runner state and orchestration errors outside `instrukt-runtime`.
- **Declared optional progressive capabilities:**
  Every capability beyond the universal live spine is explicitly declared by an
  adapter (`project/instrukt/spec/feature/runtime/adapter`, Declared optional
  progressive capabilities):
  - Idempotent start and send keys (caller-captured command/session keys and
    live reconciliation of accepted, rejected, or absent attempts).
  - Input consumption and assignment observation (live API evidence without
    reading a runtime's internal database).
  - Quota-aware model selection and readiness (model/option translation and
    background readiness/usage facts).
  - Configured agent runtimes (enabled agent harnesses and their instruction
    home directories for publication's global agent files).
  - Workspace isolation (`create_worktree`, `attach_worktree`, and typed native
    delegation capability metadata).
  - Cold text source (non-live read-only text extraction for `history` only).
  - Thread retention / archiving (`finished_threads` and guarded `archive` for
    `thread-retention`).
  - Multi-account provider isolation (Tier-1 shared-host BYOS isolation).
  - Remote peer-linking (Tier-2 operator and optional technical company-owner
    dual access).
  - Notification (`notify`).
  - Conversation link (`link`).

### Runtime declarations and composition-root wiring

- **Runtime declarations:** `instrukt-core` defines the data-only
  `RuntimeDeclaration` carrier type and startup registry so `configuration`
  validates `runner.runtime`, capability checks (such as archive support for
  thread retention), and shipped native-operation instruction catalogs without
  importing any runtime crate. Each concrete runtime adapter crate owns its
  declaration values, its shipped instruction file, and any pure static
  legacy-history record normalizer hook. `instrukt-cli` installs all compiled
  declarations once at startup (mirroring connector declarations) and fails
  closed if none is registered.
- **Adapter factory wiring:** `instrukt-cli` is the sole crate that imports
  concrete runtime adapter crates and injects the selected `RuntimeAdapter`
  (plus environment-only caller-identity and optional configured-agent-runtimes
  hooks) into `instrukt-runner`, `instrukt-company`, `instrukt-todos`, and
  `instrukt-core`.

### Concrete runtime adapters

Concrete runtime adapters live in dedicated adapter crates behind
`instrukt-runtime`. The T3 adapter (`crates/instrukt-runtime-t3`) realizes the
universal live spine (`start`/`send`/`read_state`/`caller_identity`/
`execution_root`/`list`), live reconciliation over authenticated RPC/CLI
surfaces with zero live internal-database reads, isolated non-live cold text
and archiving capabilities, Tier-1 BYOS provider instances, and Tier-2 remote
topology (`project/instrukt/spec/feature/runtime/t3`), verified against its
pinned release by provider-free contract checks
(`project/instrukt/spec/feature/runtime/t3-contract`).

### Domain events emitted

None. This context emits no domain events.

### Domain events consumed

None.

## Ubiquitous language

**Runtime adapter**
: The vendor-neutral Rust trait (`RuntimeAdapter`) in `instrukt-runtime`
  translating the universal live spine and declared optional capabilities onto
  a concrete agent runtime.

**Universal live spine**
: The six live operations every runtime adapter realizes: `start` /
  `start_or_wake`, `send`, `read_state`, `caller_identity`, `execution_root`,
  and `list`.

**Active turn**
: A runtime run currently executing in a present, active session, carrying
  validated `RunId`, `InputId`, and neutral closed opening-input data
  (text or unavailable).

**Observed turn**
: One run observed within a session's bounded live turn history, carrying
  validated `RunId`, `InputId`, and a payload-carrying `TurnState`.

**Input identity**
: The validated `InputId` (caller-captured command identity) matched against
  runtime run observations to correlate active, queued, completed, and failed
  turns without reading a runtime's internal database.

**Capability**
: An optional runtime feature beyond the universal live spine declared by an
  adapter and checked explicitly by consumers
  (`project/instrukt/spec/feature/runtime/adapter`).

**Runtime declaration**
: Published data-only metadata carried by `instrukt-core::RuntimeDeclaration`,
  authored by each concrete adapter crate and installed once at CLI startup so
  `configuration` and `workflow` validate runtime selection and capabilities
  without depending on concrete adapter crates.

**Cold text source**
: The non-live optional capability used exclusively by `history` to extract
  retained conversation text; never invoked on live operational paths.

**Runtime anti-corruption layer**
: Each concrete adapter crate translating vendor RPC, CLI, HTTP, and non-live
  storage formats into `instrukt-runtime`'s neutral types.

## Ownership boundary

- **Owns:** the vendor-neutral `RuntimeAdapter` trait, neutral session,
  presence, five-state operational observation, structured turn-correlation
  (`ActiveTurn`, `ObservedTurn`, `TurnState`, `OpeningInput`, `RunId`,
  `InputId`), and `RuntimeError` types; the progressive optional capability
  catalog; each concrete runtime's declaration values, shipped instruction
  data, and static legacy-history normalizer hook; concrete runtime adapter
  crates (starting with `instrukt-runtime-t3`, its release pin, and its
  provider-free contract suite); the zero-live-runtime-database-reads
  invariant; and non-live cold text and archiving adapter implementations.
- **Does not own:** keyed start and present-idle rollover orchestration,
  generation bookkeeping, `SessionsError`, `SessionPresence` delivery
  eligibility, todo pickup, event routing, history indexing/search, or
  thread-retention policy (owned by `runner`,
  `project/instrukt/spec/domain/runner`); company conversation bindings,
  active-speaker envelope policy, explicit replies, curated working status, or
  company hosting (owned by `company`, `project/instrukt/spec/domain/company`);
  `instrukt.yaml` storage, validation, and interpolation (owned by
  `configuration`, `project/instrukt/spec/domain/configuration`); global agent
  file rendering (owned by `publication`,
  `project/instrukt/spec/domain/publication`); todo records and review evidence
  (owned by `work`, `project/instrukt/spec/domain/work`); or flavor manifests
  and workflow derivation (owned by `workflow`,
  `project/instrukt/spec/domain/workflow`).
- **Published contracts:**
  - `instrukt-runtime` neutral trait, structured observation types, capability
    catalog, and `RuntimeError`.
  - Per-runtime `RuntimeDeclaration` values and shipped instruction catalogs
    installed into `instrukt-core` at CLI startup.
  - Environment-only caller-identity and optional configured-agent-runtimes
    implementations wired by `instrukt-cli` into `instrukt-core`.
  - Concrete adapter crates (`instrukt-runtime-t3`) wired exclusively by
    `instrukt-cli`.
- **Allowed dependencies:** `instrukt-runtime` depends only on `instrukt-core`
  (shared IDs, home paths, git worktree helpers, and neutral configuration
  value types). Each concrete adapter crate depends only on `instrukt-runtime`,
  `instrukt-core`, and `instrukt-outside`. No domain crate (`instrukt-core`,
  `instrukt-blueprint`, `instrukt-runner`, `instrukt-todos`,
  `instrukt-company`) depends on a concrete runtime adapter crate.
- **Implementation roots:** `crates/instrukt-runtime/` (neutral facade);
  `crates/instrukt-runtime-t3/` (T3 adapter, `t3.version`, shipped T3
  instructions, and contract wiring); `crates/instrukt-cli/` (sole composition
  root installing declarations and wiring concrete adapters).

## Event subjects and origins

This context emits no events.

## Use cases

- **Runtime adapter** — [project/instrukt/spec/feature/runtime/adapter](../feature/runtime/adapter.md)
  — vendor-neutral universal live spine, structured turn correlation, zero live
  runtime-database reads, and declared optional progressive capabilities.
- **T3 runtime adapter** — [project/instrukt/spec/feature/runtime/t3](../feature/runtime/t3.md)
  — T3 Code implementation over authenticated RPC/CLI/HTTP surfaces, isolated
  non-live cold text and archiving, Tier-1 BYOS provider instances, and Tier-2
  remote topology.
- **T3 dependency compatibility** — [project/instrukt/spec/feature/runtime/t3-contract](../feature/runtime/t3-contract.md)
  — deterministic provider-free checks against the pinned T3 release,
  test-double agreement, and nightly drift detection.

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — workspace crate DAG, CLI
  composition root, and runtime external-surface boundary.

## Context map position

- **Runtime → runner** — Customer / Supplier: supplies the universal live
  adapter spine, structured active/recent turn observations, live start/send
  and input-consumption evidence with zero live runtime-database reads, and
  declared optional capabilities (`project/instrukt/spec/feature/runtime/adapter`).
- **Runtime → company** — Customer / Supplier: supplies live `read_state` with
  structured active/recent turn correlation and neutral closed opening-input
  data for fail-closed active-speaker resolution and live turn tracking, plus
  idempotent start and send primitives.
- **Runtime → publication** — Customer / Supplier: supplies optional configured
  agent runtimes and their instruction homes from the selected adapter for
  rendering global agent files, through a core-declared interface wired by the
  CLI.
- **Runtime → configuration** — Published language: supplies each compiled
  runtime's `RuntimeDeclaration` data and shipped instruction catalog
  (installed once at CLI startup) for validating `runner.runtime`, capability
  prerequisites, and instruction rendering, plus environment-only
  caller-identity lookup wired by the CLI.
- **Runtime → work** — Customer / Supplier: supplies runtime session references
  stored on todos and review evidence without translating native runtime
  mechanics.
- **Runtime → workflow** — Customer / Supplier: supplies the runtime capability
  vocabulary referenced in `workflow.yaml` `requires`.
- **External agent runtimes** — Anti-corruption layer (ACL): each concrete
  adapter crate translates its runtime's RPC, CLI, HTTP, and non-live storage
  formats into `instrukt-runtime`'s neutral types.

## Known caveats

- Concrete adapter wire limits, bounded projection windows, and offline
  provider option-descriptor mappings are documented in each runtime's feature
  specification (`project/instrukt/spec/feature/runtime/t3`,
  `project/instrukt/spec/feature/runtime/t3-contract`).

## See Also

- [project/instrukt/spec/domain/context-map](context-map.md)
- [project/instrukt/design/architecture/overview](../../design/architecture/overview.md)
- [project/instrukt/spec/feature/runtime/adapter](../feature/runtime/adapter.md)
- [project/instrukt/spec/feature/runtime/t3](../feature/runtime/t3.md)
- [project/instrukt/spec/feature/runtime/t3-contract](../feature/runtime/t3-contract.md)
