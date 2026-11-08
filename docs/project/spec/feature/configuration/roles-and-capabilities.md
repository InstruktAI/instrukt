---
description: Configurable role classes and scoped capability bundles, resolved from recorded assignments, with focused command enforcement and role-aware help.
delivered_by: [instrukt-first-build]
---
# Roles and Capabilities — Spec

## What it is

A session carries the role of its recorded assignment. One role entry in
`~/.instrukt/agents.yaml` composes its class, capabilities, model selection and
optional GitHub App identity. Capabilities describe actions on objects, rather
than command allowlists. Focused roles are checked against their grants;
standing and agent roles use grants for awareness. Every caller remains subject
to the object's claim, phase and writer invariants
(`general/principle/trust-and-guardrails`).

## Canonical fields

### Configuration and composition

| Field in `agents.yaml` | Contract |
| --- | --- |
| `roles.<role>.class` | Exactly `focused`, `standing` or `agent`. Class supplies no implicit action grants. |
| `roles.<role>.can` | List of named bundles and explicit scoped grants `{action: <name>, scope: own|any}`; effective grants are their union. |
| `roles.<role>.order`, `roles.<role>.tier` | Model preferences owned by `project/instrukt/spec/feature/configuration/agent-selection`. |
| `roles.<role>.github_app` | Optional configured App reference under `project/instrukt/spec/feature/work/github-apps`; omission uses the project identity. |
| `bundles.<name>` | Named list of explicit scoped grants. Bundles contain grants, not other bundles; composition has no ordering or deny semantics. |
| `default` | Fallback class, grants and model selection: shipped class `agent`, `can: [all]`, model order and tier. An unknown named role uses this entry; its unmapped App identity is the project identity. |

The file contains the existing provider `profiles` beside these fields; there
is one `roles` map, not a parallel capabilities or GitHub role map. Every
configured entry declares its class and grants; model inputs inherit from
`default` as agent-selection specifies. The shipped `default` and
`event-handler` are unrestricted, awareness-only entries. A role absent from
the map keeps its recorded name while using `default`; it is distinguishable
from a session with no role. A roleless human session reaches every action.
Neither fallback removes object checks.

### Action catalog

Action names and each instrukt operation's action/object mapping are owned by
code. This table groups the catalog by capability family. A command's read and
write forms can map to different actions. New command paths declare their
mapping; a missing mapping is a defect, never an unclassified bypass.

| Family | Actions | Objects and current instrukt surface |
| --- | --- | --- |
| Documentation | `docs-read`, `docs-write` | Snippets and documentation workspace: `docs index|get|schema|validate`; `docs fold` mutates the target workspace. |
| Work records | `todo-read`, `todo-write` | Todos and bugs: `todo list|show|refresh`; create, update, freeze, unfreeze, remove, reactivate and known-issue suppression writes. Creation addresses the containing project. |
| Evidence | `artifact-read`, `artifact-write`, `stamp` | Todo artifacts and stamps: `todo artifacts list|read|diff`, `write`, and `todo stamp`; flag-specific phase/attempt checks remain with work. |
| Orchestration | `todo-orchestration`, `assignment-write`, `integration` | `todo prepare|work`; `todo attempt|bind` and attempt failure/no-start resolution; confirmed delivery via `todo deliver`. Native delegation is outside this catalog. |
| Session observation and recovery | `session-observe`, `history-read`, `session-recover`, `standing-bind` | `sessions list`, extracted history search/read, `sessions resolve-start`, and `events bind` reservation/binding/no-start resolution. These do not add a session messaging API. |
| Agent availability | `agents-read`, `agents-write` | Runtime/profile/model availability: `agents list|availability`, `agents status`. |
| Configuration | `config-read`, `config-write`, `secrets-write` | `config get|show|validate`, `config init|patch`, protected `secrets set|generate`; agents-file role/bundle edits are configuration writes. Secrets are never exposed by read grants. |
| Repository | `repository-read`, `repository-write`, `code-commit` | `hooks status`; `init`, `hooks install|remove`; `repo commit` in the assigned authoring workspace. Raw git is outside this catalog. |
| Publication | `publication` | `sync` and hook-triggered publication for the selected project and positions; position ownership remains mandatory. |
| Inspection | `code-check`, `eval-read` | `specs coverage|reserve|trace`, `blueprint affected|path|query|explain|status`, and `eval report`. Engine queries remain subject to their own interface. |
| Events | `events-read` | `events list|schema|status`; subscription declarations retain their documentation/configuration write boundary. |
| Installation and runner | `installation-read`, `installation-write`, `runner-execute` | `ticks status`, `install --check`; `install` convergence/withdrawal; `run`, `tick`, hook workers, blueprint workers and the internal webhook receiver execute their declared work. Internal entry paths retain object checks and receive no caller-selected role bypass. |

Scope is always authored as `own` or `any`. `any` reaches every object the
action addresses and adds no exemption to that object's invariants. `own`
reaches only objects bound by the recorded assignment. For machine-wide
objects, `own` is empty unless that machine-wide configuration or installation
responsibility is explicitly part of the recorded assignment; being in a
project does not own the machine.

### Caller and object resolution

The explicitly selected runtime adapter recognizes the native caller. For T3,
`T3CODE_THREAD_ID` yields the native id and the session reference `t3:<id>`.
The environment identifies the session, not its role. Role, todo and scope come
from that session's recorded binding, never an environment role variable, a
`--role` target argument, the current directory, a display title or git author.

For work sessions, work's `session_bindings` and attempt evidence supply the
project, durable todo identity, role, lane, attempt, worktree and branch. Native
children bind through the existing reserved attempt and matching native session
and workspace evidence. A prepare driver retains its recorded placement; a work
driver and its admitted child use the recorded shared todo workspace. An
`own` grant on todo evidence names that todo; on repository or documentation
mutation it names the assignment's recorded workspace. Rename follows durable
todo identity. Merely inspecting another todo never binds it.

For a subscribed responsibility, runner's standing assignment/generation supplies
the canonical procedure, exact subscription home, optional subject and own
thread. Project scope resolves to that recorded project home; platform scope
resolves to the recorded platform responsibility, not all projects. The
subject does not imply a todo claim. The own-thread object is the caller's
recorded native session. `session-recover: own` additionally reaches the
unresolved start key the caller's recorded assignment owns. Scheduled and other
instrukt starts retain their role and placement in their existing assignment
evidence. No second session registry or login identity is introduced.

A recognized session with no instrukt assignment is roleless; an unreadable,
contradictory or partially lost known assignment is a resolution failure, not a
roleless fallback. Recovery names the affected binding/attempt or standing key.
Assignment facts remain fixed for that assignment; reading another procedure
inline changes none. Current role configuration supplies its grants and model
preferences; configuration edits do not rewrite an assignment's recorded role,
todo, home or placement.

### Shipped bundles and role entries

The named bundles ship in the file and are editable configuration. Their shipped
composition is:

| Bundle | Scoped grants |
| --- | --- |
| `read` | `docs-read`, `todo-read`, `artifact-read`, `config-read`, `agents-read`, `events-read`, `code-check`, `eval-read`, `repository-read`, `installation-read`, each at `any`. |
| `worker` | `artifact-write`, `stamp`, `code-commit`, `docs-write`, each at `own`. |
| `review` | `artifact-write`, `stamp`, each at `own`. |
| `record` | `todo-write`, `artifact-write`, `stamp`, each at `own`. |
| `orchestrate` | `todo-orchestration`, `assignment-write`, `session-observe`, `session-recover`, `standing-bind`, each at `own`. |
| `integrate` | `integration`, `publication`, `repository-write`, `todo-write`, each at `own`. |
| `configure` | `config-write`, `secrets-write`, `installation-write`, `agents-write`, each at `any`. |
| `all` | Every catalog action at `any`. |

These entries cover every role declared by instrukt's shipped procedures.
They share the model `order`/`tier` defaults of agent-selection; the table's App
column names the identity reference, not another model/provider choice.

| Roles | Class | `can` bundles | App |
| --- | --- | --- | --- |
| `builder`, `finalizer`, `drift-analyzer`, `deferrer`, `diagnoser` | `focused` | `read`, `worker`, `record` | author |
| `prd-reviewer`, `architecture-reviewer`, `plan-reviewer`, `build-reviewer`, `commit-reviewer` | `focused` | `read`, `review` | reviewer |
| `knowledge-author`, `project-analyst`, `third-party-researcher` | `focused` | `read`, `worker`, `record` | author |
| `config-assessor` | `focused` | `read`, `configure`, `worker`, `record` | author |
| `prepare-orchestrator`, `work-orchestrator`, `intaker`, `architect`, `planner`, `diagnosis-driver`, `task-preparer`, `task-runner` | `agent` | `read`, `worker`, `record`, `orchestrate` | author |
| `integrator` | `standing` | `read`, `record`, `orchestrate`, `integrate` | project |
| `coordinator` | `standing` | `read`, `record`, `orchestrate` | project |
| `bug-hunter`, `upstream-watcher` | `standing` | `read`, `record` | project |
| `event-handler` | `standing` | `all` | project |
| `default` | `agent` | `all` | project |

A subscribed procedure with no explicit `role` receives `event-handler` at
entry and in its resolved publication metadata. The author can declare a role
and configure it as `focused` with narrower grants. Workflow role membership
applies only to workflow-dispatched procedures; a handler works independently
of todos and never needs an artificial workflow role entry.

### Enforcement, help and validation

The CLI resolves the caller and target action/object before admitting its
operation. A `focused` role needs a matching effective grant. Failure changes
no target and names the role, action and object scope plus the sanctioned
reporting path: return the required action to the dispatcher in the role's
own output/artifact. A dispatcher can perform it or change the assignment
through normal configuration/admission; a caller cannot widen its own role
by supplying target arguments. An object refusal instead names the actual
door: the holding claim, admitted attempt/phase or writer path.

For `standing` and `agent`, grants shape the help and initialized briefing;
they do not refuse an otherwise valid action outside the usual surface. The
shipped event-handler and default show the complete surface. A focused role's
help shows only permitted commands/forms, their action and scope, with object
constraints named. Briefs carry the resolved role, class, grants and assignment
scope. Help and execution use the same code-owned action mapping. Every class
and the roleless caller still pass claim, phase, shared-workspace admission,
publication ownership and single-writer-on-main checks.

Configuration validation rejects unknown classes, action names, bundle names,
scopes and malformed grants; it reports configured role entries that no
installed procedure declares. `default` and the implicit `event-handler` are
reserved fallback entries, not undeclared-procedure findings. Configuration
inspection shows the effective class, expanded grants, selection and App
reference without secrets. Procedure names remain open: an unconfigured
declared role uses `default` rather than being rejected as an unknown role.

## Use cases

### UC-RCAP1: Delegate focused work with an inspectable command scope

```gherkin
Given a developer delegates a todo to a focused role with scoped bundle grants
When its bound session reads instrukt help and records its assigned work
Then help shows the permitted surface and the assigned writes are admitted
But a command outside those grants is refused before mutation with its reporting path
And a command against another live claim still meets the object's claim gate
```

### UC-RCAP2: Publish an event responsibility without configuring a new role

```gherkin
Given a developer publishes a subscribed procedure without an explicit role
When an event activates it with todos off
Then its assignment uses event-handler with unrestricted awareness capabilities
And an unknown explicit role uses default with its recorded role name preserved
When the developer declares and configures a focused handler role
Then new assignments enforce that role's grants
```

## Known caveats

- Capabilities cover instrukt commands only. File edits, raw git, `gh` and
  native runtime tools remain governed by the runtime's permission mode.
  Per-role runtime permission mode is unspecified.
- This contract adds no agent-to-agent discussion design or session-authority
  hierarchy. Native conversation operations retain their existing contracts.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/agent-selection](agent-selection.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
