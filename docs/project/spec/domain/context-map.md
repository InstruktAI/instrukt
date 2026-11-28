---
description: Instrukt's bounded contexts, their module placement and directed relationships.
---
# Context Map — Spec

## What it is

The semantic map of instrukt: eleven bounded contexts with separate ownership,
ten assembled into modules with downward dependencies and one, distribution,
outside the binary. Contexts own behavior;
modules determine which capabilities are enabled. The map indexes their domain
contracts and the designs governing their relationships.

## Ownership boundary

This map indexes the bounded contexts and their relationships; each context's
domain spec owns its behavior. The map is not an additional bounded context.

## Canonical fields

### Bounded contexts


| Context | Owns | Domain spec |
| --- | --- | --- |
| docs | Snippet model, validation, index reading and retrieval | `project/instrukt/spec/domain/docs` |
| configuration | Layered settings, company homes with their own settings and secrets, role classes/scoped bundles/model selection, App declarations, secrets, validation, migration and interpolation | `project/instrukt/spec/domain/configuration` |
| publication | Committed-state publication, index patching, rendering of the global agent files, of committed `AGENTS.md` files and of a company's plugin with its catalog entry, agent settings, adoption claims, editor schemas and the `instrukt status` report | `project/instrukt/spec/domain/publication` |
| repository | Repository initialization, git hooks and commit composition | `project/instrukt/spec/domain/repository` |
| specs | Use-case inventory, test bindings and reservation | `project/instrukt/spec/domain/specs` |
| blueprint | Revision-keyed structural graphs, graph queries and use-case trace joins | `project/instrukt/spec/domain/blueprint` |
| work | Local work record as its only copy, one-way GitHub reflection with people's controls read back once, App credential/actor boundary, artifacts and stamps | `project/instrukt/spec/domain/work` |
| workflow | Configured state derivation, transitions and integration gate | `project/instrukt/spec/domain/workflow` |
| runner | Runtime dispatch, standing agents, event sources, scheduled ticks, agents' MCP access, per-computer searchable history and the container each company runs in on a host | `project/instrukt/spec/domain/runner` |
| company | Connectors, the conversations they carry bound to agent sessions, who is answered, the answers posted back and the speaker of each turn | `project/instrukt/spec/domain/company` |
| distribution | The release, the release installer and the instrukt cockpit skill | `project/instrukt/spec/domain/distribution` |


### Modules

| Module | Contexts | Availability and dependencies |
| --- | --- | --- |
| core | docs, configuration, publication, repository, specs | Always on; independent of optional modules |
| blueprint | blueprint | Switchable; uses core repository information and a graph engine |
| todos | work, workflow, runner | Switchable work record and workflow; GitHub and runtime adapter required; runner autostart or manual concerns todos only |
| history | runner's extraction, retained conversations and merged search | On by default; local operations need no Redis; merged search independently demands cluster Redis, with no events dependency |
| events | runner's event sources and standing-agent dispatch | Independently switchable; runtime adapter required; shares the single runner process |
| conversations | company | Given by a company home's declared connector; runtime adapter required; served by the long-lived `instrukt run` |

Distribution is in no module: it is release tooling and the skill beside
the binary, reaching it only through its command surface.

Events are a module switch within the runner's semantic boundary, not a tenth
bounded context. The runner's work follows the enabled modules; events do not
require todo autostart. A flavor declares its module requirements through
`requires`. Eval has no module or switch: stamps carry model, version and eval
fingerprint; reporting uses `instrukt eval report` and the flavor's report
procedure. There is reportable work only where work passes through the record.

A report covering other computers reaches them through the runner's fleet query
over the cluster connection each computer already has, whether or not history
is on there; it adds no module, switch or connection.

### Context relationships

Customer / Supplier rows name the supplier first and the customer second.
The labels follow `sdlc/spec/domain-spec`; direction describes contract use,
not permission for an upstream module to depend on its consumer.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → docs | Customer / Supplier | Read-time configuration interpolation through the CLI |
| configuration → publication | Customer / Supplier | Project slug, runtime choice and configuration migration operation |
| configuration → repository | Customer / Supplier | Project slug, runtime choice, role/App references and non-bot author settings |
| blueprint → runner | Customer / Supplier | Blueprint-owned release pin and tool lifecycle consumed by module installation |
| configuration → blueprint | Customer / Supplier | Module switch and engine selection |
| configuration → work, workflow, runner | Customer / Supplier | Board, approval gates, runtime choice, role classes/scoped grants/model selection, App configuration, runner settings and company homes |
| configuration → company | Customer / Supplier | Company homes with their settings, members, resolved secrets and declared connectors |
| company → configuration | Published language | Each connector kind's declaration, which configuration validates a company home's `connectors:` against (`project/instrukt/spec/domain/configuration`) |
| runner → company | Customer / Supplier | Runtime adapter, native conversation source and turn relay, keyed start, identity envelopes and the state database handle |
| company → runner | Customer / Supplier | Conversation hosting in `instrukt run` and the conversation assignment lookup, declared by runner, implemented by company and wired by the CLI; a connector's direct message to one person, which MCP sign-in sends its links through |
| work → company | Customer / Supplier | The state database's file, writer lock and migrations, holding the conversation tables |
| work, runner, company → CLI admission | Published language | Owner-supplied todo attempts, standing/scheduled assignment facts and conversation assignments for the recognized caller; core-declared lookup wired by the CLI |
| work GitHub boundary → runner, repository | Customer / Supplier | Installation credential and verified bot-actor interfaces, composed by CLI wiring without reversing crate dependencies |
| docs → publication | Customer / Supplier | Snippet identity, validation, `index.yaml` and `schema.yaml` model, and reference model |
| publication → docs | Customer / Supplier | Home corpus and owner-patched global index consumed by retrieval; whether a repository's `docs/index.yaml` is its company index, consumed by validation |
| publication → work | Customer / Supplier | Eval fingerprint recorded per published commit, copied into stamps |
| publication → repository | Customer / Supplier | Sync and committed-entrypoint rendering invoked by repository hook sections |
| runner → publication | Customer / Supplier | Configured agent runtimes and their homes from the selected runtime's adapter, for the global agent files; core-declared interface wired by the CLI |
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
| publication → workflow | Customer / Supplier | Published flavor manifest (`workflow.yaml`: orchestrator, requires, roles) from the selected domain's `docs/domain/<x>/workflow/` |
| publication → runner | Customer / Supplier | Project checkout paths recorded in publication state |
| repository → workflow | Customer / Supplier | Candidate integration branch and exact-commit main advancement |
| runner → workflow | Published language | `runner.ci_completed` wakes the integrator to assess candidates; `runner.main_failing` and `runner.dependency_failing` supply their respective failed-check evidence |
| repository → publication | Customer / Supplier | Hook-section state read by `instrukt status` |
| publication → distribution | Published language | `instrukt status`, the state the cockpit skill reads |
| configuration, repository, runner → distribution | Published language | `instrukt config`, `instrukt init` and `instrukt install`, the commands the cockpit runs |

Automatic review adds these crossings; their composition is the review gate in
`project/instrukt/design/architecture/overview`.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → runner, work, workflow | Customer / Supplier | The per-project automatic-review opt-in |
| work → configuration, runner | Customer / Supplier | The readiness of the project identity to observe and of the reviewer identity to publish, consulted where the opt-in is admitted and by installation inspection |
| work → runner | Customer / Supplier | Whether a pull request is a todo's, whether a commit carries a verdict and which review evidence is current for it; recording or withdrawing a derived approval and publishing outstanding verdicts through work's command contract |
| runner → work | Customer / Supplier | A commit's review requests, each with its number, its standing (queued, selected, handed over to a named session, or ended with its disposition), the session it reached and its pinned base; which input opened the turn a named session has in progress when it offers a revision; and a named session's conversation. All three are consulted when a head verdict is admitted |
| work → workflow | Customer / Supplier | Which review evidence is current for a commit and whether a candidate's derived approval is one GitHub does not show yet: the only source of a non-todo head's approval and of exemptions in coverage, and with the review status the source of a candidate's result |
| workflow → runner | Customer / Supplier | An active candidate's review coverage, derived from the repository, work's answer and the runner's red observation of the canonical head |

Fleet attention adds these crossings
(`project/instrukt/spec/feature/runner/fleet-attention`):

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| runner → work | Customer / Supplier | The fleet-attention contract, called inside work's transaction for each change it makes on this computer to a todo's status, hold or attempts, with that change's facts; the runner decides the conditions and appends their events |
| work → runner | Customer / Supplier | A project's active todos with their status and holds, read when a raise is admitted and when a runner-observed condition opens an attention episode |
| workflow → runner | Customer / Supplier | Whether a claimed todo waits on a person, carried by the existing assignment and read when a claimed session's input wait is observed |

The evaluation report across computers adds these crossings
(`project/instrukt/spec/feature/work/eval-report`). The runner owns the fleet
query and the fleet membership record (`project/instrukt/spec/domain/runner`);
work owns what a computer answers, which computers a report covers and how
their answers pool.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| runner → work | Customer / Supplier | The fleet query, carrying the evaluation report's request to the computers it covers and returning each answer or why there is none, and the fleet membership record, read by the report on the asking computer |
| work → runner | Customer / Supplier | This computer's answer to an evaluation request, through the evaluation-answer interface the runner declares, work implements and the CLI wires; the runner's responder returns it unread |
| publication → work | Customer / Supplier | The projects a home publishes and their checkout paths, read by the evaluation report on the asking computer and on each answering one |

A requested review (`project/instrukt/spec/feature/runner/requested-review`)
adds two crossings and is no part of the review gate: it emits no event and the
runner holds no review request for it.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| work → runner | Customer / Supplier | Whether a commit's head verdict stands on this computer, with the session that recorded its current version, or was handed off, with the export that handed it off; the pinned base work admits the commit's verdict against; and, for that project, which commit's standing verdict has a version with a given report digest; consulted when a requested review is admitted |
| runner → work | Customer / Supplier | The requested-review verdict-notice queuing, called by `instrukt review record` once its evidence write has closed, with the recorded version |


Worktree reclamation adds these crossings; its contract is
`project/instrukt/spec/feature/runner/worktree-reclamation`.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → runner | Customer / Supplier | The project's admitted declaration of regenerable output |
| workflow → runner | Customer / Supplier | Each todo worktree's standing on the computer (delivered, idle or under way) and what a delivered todo still owes of its worktree and work branch |
| work → runner | Customer / Supplier | Through work's command contract: the section of the work record in which no role attempt opens on a todo's worktree, and the release of a delivered todo's worktree and work branch |

Adding a company adds these crossings; its contract is
`project/instrukt/spec/feature/configuration/company-homes` (Generation), and
repository's company home adoption carries it out.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| work GitHub boundary → configuration | Customer / Supplier | Whether a login names an organisation, a personal account or nothing, through an interface configuration declares and the CLI wires |
| work GitHub boundary → repository | Customer / Supplier | Creating the company's private repository in the owning organisation, making its GitHub administrator an administrator and setting its default branch, through an interface core declares and the CLI wires |
| configuration → repository | Customer / Supplier | The generation decision for a company's key, the skeleton's project layer and ignore lines, and the operation that writes the home's settings document |
| publication → repository | Customer / Supplier | The skeleton's rendered `AGENTS.md`, skill and catalog, its project layer's modeline, and the sync of the new home |
| publication → docs | Customer / Supplier | Whether a repository is a company's, consumed by validation, which then reads nothing under its `docs/` |

Carrying a company's knowledge to its default branch adds these crossings;
their contracts are `project/instrukt/spec/feature/runner/pull-request-review`
and `project/instrukt/spec/feature/workflow/integration-gate` (A company
home's candidate).

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| company → repository | Customer / Supplier | The proposal recording the `post-commit` hook run invokes, through an interface core declares and the CLI wires |
| company → runner | Customer / Supplier | The proposal lookups runner declares and company implements: whether a head is an admitted proposal, the unestablished entries on remote-tracking branches and the store's condition |
| work GitHub boundary → runner | Customer / Supplier | The readiness of the proposal pull-request credential, whether any pull request has a commit as its head, and opening a pull request for a branch under the proposal pull-request credential |
| configuration → runner | Customer / Supplier | A company repository's company identity, whose finding withholds the home's event responses |
| runner → workflow | Published language | `runner.proposal_approved` wakes the company's integrator to integrate an approved admitted proposal's exact head |
| configuration → publication | Customer / Supplier | The company identity a company repository's committed project layer declares, with its finding |
| publication → workflow | Customer / Supplier | Publication's one operation rendering a tree's agent material, from which candidate coverage reproduces a rendering commit |

Thread retention adds these crossings; its contract is
`project/instrukt/spec/feature/runner/thread-retention`.

| Supplier → customer | Relationship | Contract crossing the boundary |
| --- | --- | --- |
| configuration → runner | Customer / Supplier | The computer's admitted retention window, refused where configuration enters when the selected runtime cannot archive threads |
| work → runner | Customer / Supplier | The native sessions bound to, or holding the execution claim of, a project's todos that have not reached a terminal outcome, delivered or withdrawn, read from work's local record |

Workflow and runner coordinate integration without making the workflow engine
event-driven: it computes state and emits few events; the runner receives
external events and wakes agents. GitHub checks guard main; the integrator
interprets their results. Core does not require blueprint or todos: trace joins
belong to blueprint. Core declares the todo-existence, flavor-role lookup and
hook-generation interfaces; the CLI wires work and workflow when todos is
enabled, and blueprint when blueprint is enabled.

External models are translated through **Anti-corruption layer (ACL)**
relationships: work's GitHub boundary translates the record one way into
issues, artifacts, the work branch, its pull request and reviews, and people's
controls back into attributed record changes; blueprint's engine adapter translates the engine graph into its
node-link contract; runner's runtime adapter and event-source boundary translate
session operations, native thread streams and external notifications; the
company context's connectors translate a channel provider's messages into
conversation turns and answers, at the connector interface company defines.

Distribution is **Conformist** to the Agent Skills format,
the `skills` command and GitHub's release assets.

Publication is **Conformist** to Claude's plugin, skill and plugin catalog
formats, for the company plugin it renders into a company's repository
(`project/instrukt/spec/domain/publication`).

## Governing designs and decisions

- `project/instrukt/design/architecture/overview` — module crates, home layout, publication and retrieval flows, push-triggered graphs and CI-gated integration; no consumer daemon beside the runner.
- `project/instrukt/policy/testing` — repository test authorship and validation ruling.
- `project/instrukt/design/architecture/outside-gate` — internal daemon utility through which long-lived retrying loops (connectors, relay, background pollers) share cooldowns, wait, and report outages once per episode.

## Ubiquitous language

Context and module identities and relationship labels are defined in
Canonical fields and in the domain specs indexed there.

## Event subjects and origins

The context map emits no events.

## Use cases

Each bounded context's domain spec indexes its feature use cases.

## Context map position

This map records the relationships across all bounded contexts.

## Known caveats

- One build run covers every module in order — core, blueprint, todos, events, history — each with its own acceptance.
- Enabled events use tick-read sources and opt-in signed webhook ingress
  under the runner event contracts. Reactions activate agents, never script-run
  event actions or event-bound maintenance.
