---
description: Instrukt's nine bounded contexts, their module placement and directed relationships.
delivered_by: [instrukt-first-build]
---
# Context Map — Spec

## What it is

The semantic map of instrukt: nine bounded contexts with separate ownership,
assembled into modules with downward dependencies. Contexts own behavior;
modules determine which capabilities are enabled. The map indexes their domain
contracts and the designs governing their relationships.

## Canonical fields

### Bounded contexts

| Context | Owns | Domain spec |
| --- | --- | --- |
| docs | Snippet model, validation, index reading and retrieval | `project/instrukt/spec/domain/docs` |
| configuration | Layered settings, validation, migration and interpolation | `project/instrukt/spec/domain/configuration` |
| publication | Committed-state publication, index patching, rendering of the global agent files and project `AGENTS.md`, agent settings, adoption claims and editor schemas | `project/instrukt/spec/domain/publication` |
| repository | Repository initialization, git hooks and commit composition | `project/instrukt/spec/domain/repository` |
| specs | Use-case inventory, test bindings and reservation | `project/instrukt/spec/domain/specs` |
| blueprint | Revision-keyed structural graphs, graph queries and use-case trace joins | `project/instrukt/spec/domain/blueprint` |
| work | Local-leading work record, GitHub reflection, artifacts and stamps | `project/instrukt/spec/domain/work` |
| workflow | Configured state derivation, transitions and integration gate | `project/instrukt/spec/domain/workflow` |
| runner | Runtime dispatch, standing agents, event sources and scheduled ticks | `project/instrukt/spec/domain/runner` |

### Modules

| Module | Contexts | Availability and dependencies |
| --- | --- | --- |
| core | docs, configuration, publication, repository, specs | Always on; independent of optional modules |
| blueprint | blueprint | Switchable; uses core repository information and a graph engine |
| todos | work, workflow, runner | Switchable work record and workflow; GitHub and runtime mapping required; runner autostart or manual concerns todos only |
| events | runner's event sources and standing-agent dispatch | Independently switchable; runtime mapping required; shares the single runner process |

Events are a module switch within the runner's semantic boundary, not a tenth
bounded context. The runner's work follows the enabled modules; events do not
require todo autostart. A flavor declares its module requirements through
`requires`. Eval has no module or switch: stamps carry model, version and eval
fingerprint; reporting uses `instrukt eval report` and the flavor's report
procedure. There is reportable work only where work passes through the record.

### Context relationships

Customer / Supplier rows name the supplier first and the customer second.
The labels follow `sdlc/spec/domain-spec`; direction describes contract use,
not permission for an upstream module to depend on its consumer.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → docs | Customer / Supplier | Read-time configuration interpolation through the CLI |
| configuration → publication | Customer / Supplier | Project slug, runtime mapping and configuration migration operation |
| configuration → repository | Customer / Supplier | Project slug, runtime mapping and agent author settings |
| configuration → blueprint | Customer / Supplier | Module switch and engine selection |
| configuration → work, workflow, runner | Customer / Supplier | Board, approval gates, runtime mapping and runner settings |
| docs → publication | Customer / Supplier | Snippet identity, validation, `index.yaml` and `schema.yaml` model, and reference model |
| publication → docs | Customer / Supplier | Home corpus and owner-patched global index consumed by retrieval |
| publication → work | Customer / Supplier | Eval fingerprint recorded per published commit, copied into stamps |
| publication → repository | Customer / Supplier | Sync invoked by repository hook sections |
| blueprint → repository | Customer / Supplier | Graph generation invoked by hooks for a push or pull advancing main; CLI wires the core-declared interface |
| docs → specs | Customer / Supplier | Feature-spec use-case inventory and pending markers |
| specs → blueprint | Customer / Supplier | Use-case inventory and test bindings joined to the graph for traces (trace is blueprint's) |
| configuration → specs | Customer / Supplier | Project slug identifying the repository inventory |
| workflow → docs | Customer / Supplier | Declared flavor roles validated through a core interface when todos is enabled |
| work → docs | Customer / Supplier | Existence of todo slugs named by pending docs, only with todos enabled |
| work → repository | Customer / Supplier | Todo existence through a core-declared interface wired by the CLI when todos is enabled |
| work → workflow | Customer / Supplier | Artifacts, stamps and facts from which state is derived |
| workflow → runner | Customer / Supplier; Published language | Computed state, transitions and finalize-ready event used for dispatch |
| work → runner | Customer / Supplier | Work reads and command-contract writes: claims and reflection flushes, last-fired stamps, automatic holds, expired-record purge and per-role session bindings |
| runner → work | Customer / Supplier | Event-log append used when the finalize-ready stamp records `workflow.finalized` |
| docs → runner | Customer / Supplier | Procedures and their event subscriptions |
| publication → workflow | Customer / Supplier | Published flavor definitions, roles, actions and templates from the selected domain's `docs/domain/<x>/workflow/` |
| publication → runner | Customer / Supplier | Project checkout paths recorded in publication state |
| repository → workflow | Customer / Supplier | Candidate integration branch and exact-commit main advancement |
| runner → workflow | Published language | `runner.ci_completed` and `runner.main_ahead` wake the integrator to assess candidates and direct commits |

Workflow and runner coordinate integration without making the workflow engine
event-driven: it computes state and emits few events; the runner receives
external events and wakes agents. GitHub checks guard main; the integrator
interprets their results. Core does not require blueprint or todos: trace joins
belong to blueprint. Core declares the todo-existence, flavor-role lookup and
hook-generation interfaces; the CLI wires work and workflow when todos is
enabled, and blueprint when blueprint is enabled.

External models are translated through **Anti-corruption layer (ACL)**
relationships: work's GitHub reflection boundary translates issues, artifacts
and stamps; blueprint's engine adapter translates the engine graph into its
node-link contract; runner's runtime mapping and event-source boundary translate
session operations and external notifications. These are boundary locations,
not additional contexts.

### Cross-cutting designs and rulings

- `project/instrukt/design/architecture/overview` — module crates, home layout, publication and retrieval flows, push-triggered graphs and CI-gated integration; no daemon.
- `project/instrukt/policy/testing` — repository test authorship and validation ruling.

## Known caveats

- One build run covers every module in order — core, blueprint, todos, events — each with its own acceptance.
- Events come from reading state on each tick; webhook receipt is not part of
  this build (see the runner's event-activation spec).
