---
description: Configurable role classes and scoped capability bundles, resolved from recorded assignments, with focused command enforcement and role-aware help.
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
is one `roles` map, not a parallel capabilities or GitHub role map. Missing
`class` and `can` fields resolve independently from the shipped entry
for a known role. Unknown configured roles inherit those fields from the effective
`default`; missing fields on `default` inherit the shipped default. Explicit
values, including an empty `can` list, win. Explicit malformed/null values are
validation errors, not missing fields. This read-time resolution preserves the
edited file. Shipped named bundle definitions supply omitted definitions needed
by resolved grants; an explicitly configured bundle wins.

The default role of conversation sessions is `company-agent`; a named connector
may declare a different `role` in its company home's `company.yaml`, which is
recorded on the session's `connected_conversations` binding and returned by
caller assignment lookup. An agents file of schema
version 2 converts to version 3 on the owned migration path
(`project/instrukt/spec/feature/configuration/owned-files`): its
`roles.conversation` entry becomes `roles.company-agent` with every field as
it was, and nothing else changes. A version-2 file that holds both entries is
refused with its source intact, naming both, until one is removed.

The standing project coordinator's role is `sdlc-coordinator`. An agents file
of schema version 3 converts to version 4 on the same owned migration path: a
configured `roles.coordinator` entry becomes `roles.sdlc-coordinator` with
every field as it was, and nothing else changes; a version-2 file takes both
conversions. The file
overrides everything it names
(`project/instrukt/spec/feature/configuration/agent-selection`), and a
person's stated choices and restrictions are migrated, never discarded
(`project/instrukt/policy/checks-and-guards`); the conversion takes no value
from the shipped defaults. A file that holds both entries is refused with its
source intact, naming both, until one is removed. A file that names neither
entry takes the new version and no other change.

Every effective entry has a class and grants; model inputs inherit from
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
| Work records | `todo-read`, `todo-write`, `bug-report` | Todos and bugs: `todo-read` is `todo list|show`, `bugs list` and `bugs suppressions`; `todo-write` is create, update, freeze, unfreeze, remove, reactivate and known-issue suppression writes, and creation addresses the containing project; `bug-report` is `bugs report`, with or without `--approved`, and addresses the project the report names (`project/instrukt/spec/feature/work/todos`); at `own` that is the assignment's project. |
| Evidence | `artifact-read`, `artifact-write`, `stamp` | Todo artifacts and stamps: `todo artifacts list|read|diff`, `write`, and `todo stamp`; flag-specific phase/attempt checks remain with work. |
| Orchestration | `todo-orchestration`, `assignment-write`, `integration` | `todo work`; `todo attempt|bind` and attempt failure/no-start resolution; confirmed delivery via `todo deliver`. Native delegation is outside this catalog. |
| Session observation and binding | `session-observe`, `history-read`, `standing-bind`, `conversation-link` | `sessions list`, extracted history search/read, `events bind` reservation/binding/no-start resolution. `discuss open|close` manage the caller's conversation link; opening/final messages use native send. These do not add a general session messaging API. |
| Agent availability | `agents-read`, `agents-write` | Runtime/profile/model availability: `agents list|availability`, `agents status`. |
| Configuration | `config-read`, `config-write`, `secrets-write` | `config get|show|validate|schema`, `config init|patch`, protected `secrets set|generate`; agents-file role/bundle edits are configuration writes. Secrets are never exposed by read grants. |
| Repository | `repository-read`, `repository-write`, `code-commit` | `hooks status`; `init`, `hooks install|remove`; `repo commit` in the assigned authoring workspace. Raw git is outside this catalog. |
| Publication | `publication` | `sync` and hook-triggered publication for the selected project and positions; position ownership remains mandatory. |
| Inspection | `code-check`, `eval-read` | `specs coverage|reserve|trace`, `blueprint affected|path|query|explain|status`, and `eval report`. Engine queries remain subject to their own interface. |
| Events | `events-read`, `attention-raise` | `events list|schema|status`; `attention-raise` is `events raise`, addressing the project the raise belongs to, at `own` the assignment's project (`project/instrukt/spec/feature/runner/fleet-attention`). Subscription declarations retain their documentation/configuration write boundary. |
| Tool servers | `mcp-access` | `mcp list|list-tools|call`: the servers the caller's scope shows and calls to their tools, under `project/instrukt/spec/feature/runner/mcp-access`. Which login a call carries is decided there, never by this grant. `own` reaches the servers of the assignment's own company and the machine-wide open servers that company's sessions see; a machine-wide server that carries a login is a machine-wide object. `any` lifts that restriction and changes nothing in which servers the caller sees, which company it may name or which login a call carries. |
| Company conversations | `company-reply` | `company reply`: queue an explicit reply to an admitted conversation of the caller's active company connector binding (`project/instrukt/spec/feature/company/conversations`). `own` reaches the caller's own bound conversation (or, under `session_scope: company`, any admitted conversation on that `(company, connector)`); object checks always require an active company conversation binding and never cross companies or connectors. |
| Installation and runner | `installation-read`, `installation-write`, `runner-execute` | `ticks status`, `install --check`; `install` convergence/withdrawal; `run`, `tick`, hook workers, blueprint workers and the internal webhook receiver execute their declared work. Internal entry paths retain object checks and receive no caller-selected role bypass. |


Review evidence belongs to the Evidence family: `instrukt review record` and
`instrukt review exempt` are `stamp`, and `instrukt review show` is
`artifact-read`, addressing the review evidence of the project the caller
stands in. The grant reaches the command; what a caller may record is decided
by the evidence's own admission invariants, which bind every caller alike
(`project/instrukt/spec/feature/work/github-apps`). `instrukt review export` and `instrukt review import` move a
computer's review evidence for a project and are `installation-write`,
addressing this computer's installation
(`project/instrukt/spec/feature/work/github-apps`).

`instrukt repo reclaim` belongs to the Repository family: it is `code-commit`,
so the roles that commit in a todo's worktree are the roles that reclaim it.
Its object is the workspace the caller's assignment records, wherever the
caller stands: `own` reaches it for every caller whose assignment records
one. The grant reaches the command; which session may reclaim at a given
moment is decided by the worktree's own writer admission, which binds every
caller alike (`project/instrukt/spec/feature/runner/worktree-reclamation`).

`instrukt review start` asks for a requested review
(`project/instrukt/spec/feature/runner/requested-review`) and is
`assignment-write`, addressing the project the caller stands in: it records an
assignment and starts the session that works it, as `todo attempt` reserves
one, and records no review evidence. The shipped `orchestrate` bundle carries
it; focused roles without that bundle, the reviewers and builders among them,
are refused it. The requested reviewer it starts resolves to the
`commit-reviewer` role, the project and its placement from the runner's
requested-review record, and its `stamp` grant at `own` reaches that project's
review evidence.

`instrukt init --company <key>` belongs to the Repository family: it is
`repository-write`, and its object is the company's home, a machine-wide
object, so only `any` reaches it. Adding a company is the operator's: a
session whose recorded assignment names a company, a company conversation's
among them, is refused it whatever its class, naming the operator's door,
and nothing is created
(`project/instrukt/spec/feature/configuration/company-homes`, Generation).

<!-- planned-change:company-home-integration -->
`instrukt docs units` belongs to the Documentation family: it is `docs-read`,
as the read forms of the renderings are — `docs render --check`,
`docs plugin --check` and `docs plugin --validate`. Its object is the
checkout the caller stands in, which it reads and never writes; like those
read forms, it needs no home, publication, configuration layer, database or
network, so a fresh clone with the binary is enough
(`project/instrukt/spec/feature/publication/compressed-units`).
<!-- change:company-home-integration -->
`instrukt docs units` belongs to the Documentation family: it is `docs-read`,
as the read forms of the renderings are — `docs render --check`,
`docs render --plugin --check`, `docs plugin --check` and
`docs plugin --validate`. Its object is the checkout the caller stands in,
which it reads and never writes; like those read forms, it needs no home,
publication, configuration layer, database or network, so a fresh clone with
the binary is enough
(`project/instrukt/spec/feature/publication/compressed-units`).
`docs render --plugin`, like `docs render` and `docs plugin`, is `docs-write`.
<!-- /planned-change:company-home-integration -->

Scope is always authored as `own` or `any`. `any` reaches every object the
action addresses and adds no exemption to that object's invariants. `own`
reaches only objects bound by the recorded assignment. For machine-wide
objects, `own` is empty unless that machine-wide configuration or installation
responsibility is explicitly part of the recorded assignment; being in a
project does not own the machine.

`instrukt hooks run`, the call in each of instrukt's hook sections
(`project/instrukt/spec/feature/repository/hook-sections`), belongs to the
Repository family: it is `code-commit`, addressing the working tree the hook
runs in, so `own` reaches it in the workspace the caller's assignment records.
The detached `hooks worker` a post-hook starts remains `runner-execute`, and
the sync it runs remains `publication`.

### Caller and object resolution

The selected runtime adapter recognizes the native caller. For T3,
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
recorded native session. Obsolete `session-recover` grants migrate away with
the removed commands; every other configured grant and restriction is retained. Scheduled and other
instrukt starts retain their role and placement in their existing assignment
evidence. No second session registry or login identity is introduced.

A recognized session with no instrukt assignment is roleless; an unreadable,
contradictory or partially lost known assignment is a resolution failure, not a
roleless fallback. Recovery names the affected binding/attempt or standing key.
When one native session is reused across sequential attempts of the same
project, todo, role and worker class, an active unreleased attempt supersedes
earlier released attempts and released history resolves to the highest attempt;
contradictory binding facts or attempts spanning different active assignments
or scopes remain resolution failures.
Assignment facts remain fixed for that assignment; reading another procedure
inline changes none. Current role configuration supplies its grants and model
preferences; configuration edits do not rewrite an assignment's recorded role,
todo, home or placement.

### Shipped bundles and role entries

`crates/instrukt-core/src/config/shipped/agents.yaml` is the single source of
truth for the shipped named bundles, role entries, classes, grant compositions,
GitHub App references, and model `order`/`tier` defaults (editable in
`~/.instrukt/agents.yaml` and inspectable via `instrukt agents list`). Its
entries cover every role declared by instrukt's shipped procedures, the
`company-agent` role the runner gives a conversation session
(`project/instrukt/spec/feature/company/conversations`), and the reserved
`default` and `event-handler` fallbacks. Model `order` and `tier` defaults
follow `project/instrukt/spec/feature/configuration/agent-selection`; `github_app`
names only the GitHub identity reference, never model selection.

A company connector may declare `role: technical-coordinator`; its sessions
then start from `general/procedure/technical-coordinator`, the procedure
declaring that role, rather than from company assistance. Because a company
conversation binding carries no bound project or todo, `own`-scoped work-record
grants do not match a target project on their own; as a `standing` role, the
session carries its grants as declared focus and relies on object-level checks
rather than focused-class command refusals.

A company's agent reports defects of the system it works through
(`general/procedure/bug-reporting`) into the project that owns the defect on its
computer, which its own assignment never binds; `report` therefore reaches any
project at `any`. It is the company agent's only work-record write: it holds no
`todo-write`, so it creates, changes and withdraws no other todo. What a
report files is the work context's
(`project/instrukt/spec/feature/work/todos`).

`sdlc-coordinator` is the standing role of a project's fleet coordination. Its
`own` scope is the project home of its standing assignment. Its
fleet-attention grants (`project/instrukt/spec/feature/runner/fleet-attention`)
are `read`, reaching the events and records a condition names; `record`,
reaching that project's todos with the holds, dependencies and withdrawals that
answer a condition; and `orchestrate`, reaching that project's sessions.
`report` files a defect of the system it works through into the project that
owns the defect, as it does for a company's agent.

A raise is `attention-raise`, carried by `orchestrate`, so every orchestrating
and standing role that orchestrates holds it. A shipped focused worker or
reviewer holds no `attention-raise`: its raise is refused, and it returns the
condition to the orchestrator that dispatched it in its own output, which
raises it when it cannot resolve it.

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
reserved fallback entries and `company-agent` is the role the runner gives
conversation sessions; none of the three is an undeclared-procedure finding. Configuration
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

### UC-RCAP3: A company's agent may report defects and change no other work

```gherkin
Given a company's agent in a conversation
When it reads what it may do
Then reporting a defect into any project on its computer and reading which defects are already reported are among its actions
And creating, changing or withdrawing any other work is not
```

### UC-RCAP4: A focused author's hook runs are admitted in its own workspace

```gherkin
Given a focused author holding code-commit at own, bound to its assigned worktree, with instrukt's hooks installed
When it commits there
Then the hook runs its commit triggers are admitted
But a hook run it starts outside its assigned worktree is refused before it runs, with its reporting path
```

## Known caveats

- Capabilities cover instrukt commands only. File edits, raw git, `gh` and
  native runtime tools remain governed by the runtime's permission mode.
  Per-role runtime permission mode is unspecified.
- This contract adds no agent-to-agent discussion design or session-authority
  hierarchy. Native conversation operations retain their existing contracts.
- `company-agent` is a `standing` role, so its grants shape its help and
  briefing and refuse nothing: an instrukt command outside `read`, `tools` and
  `report` is held back by its procedure
  (`general/procedure/company-assistance`), not refused by this layer.
- A `focused` role without `code-commit`, such as a reviewer, is refused the
  hook run a commit of its own triggers, and that commit is made without what
  instrukt generates. Every shipped `focused` role that commits holds
  `code-commit` through `worker`.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/agent-selection](agent-selection.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/work/state-database](../work/state-database.md)
- [project/instrukt/spec/feature/runner/procedure-standing-agent](../runner/procedure-standing-agent.md)
- [general/principle/trust-and-guardrails](../../../../general/principle/trust-and-guardrails.md)
