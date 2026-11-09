---
description: "`instrukt.yaml` in a global and a project layer — which key lives where, how the layers combine, validation, schema versions and migration."
delivered_by: [instrukt-first-build]
---

# Layered Configuration — Spec

## What it is

`instrukt.yaml` uses one schema read in two layers,
like git's user and repository configuration. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and what only the
machine may decide; the project layer (`instrukt.yaml` at the repository root)
holds what the project declares, and overrides the global layer where a key
may live in both. Validation knows, per key, which layer it may live in. Both
layers carry `schema_version`, and older documents are migrated.

## Canonical fields

`runner.runtime` chooses a compiled adapter. Dispatch preferences are checked
against the selected runtime's native and adapter capabilities, never executable
configuration. Agent-native instructions are configuration prose rendered at read time;
they are not adapter command templates.

### History enablement

`modules.history` is a boolean allowed in both layers and defaults on; an
explicit project value overrides the global value. Operational enablement is
owned by `project/instrukt/spec/feature/runner/history`; infrastructure demand
is owned by `project/instrukt/spec/feature/runner/module-installation`.

### Event enablement

The effective `modules.events` switch is authoritative. Subscriptions remain in
published documentation while off and produce no activation. A nonempty valid
project `webhooks:` map adds receiver demand only while events are enabled; a
declaration with events off produces a plain validation finding naming that
switch. An empty map adds no receiver demand. Source declarations never silently
enable events. Installation and withdrawal are owned by
`project/instrukt/spec/feature/runner/module-installation`; these schema rules install nothing.

### Layer rules

| Key | Layer | Meaning owned by |
| --- | --- | --- |
| `schema_version` | each document | configuration |
| `agents.<name>.git_author` | global only | commit composition |
| `modules: {blueprint, todos, events, history}` (booleans; history defaults on, others off; core has no switch) | both (project overrides global) | each module |
| `events.watch: [{remote, ref}]` | project only | runner |
| `webhooks.<domain>` | project only; required `secret`, optional `previous_secret` references, `verify` (native default, github, hmac-sha256), generic `header`, exclusive `name_from` or `name` | runner intake |
| `ingress.computer`, `ingress.port` | global only; stable existing machine ID and loopback port | runner intake |
| `redis.cluster`, `redis.local` | global only; `url`, optional `username`/`password` references and connection limits/timeouts | runner transport |
| `ticks` | global only | the runner |
| `blueprint.engine` | both; the project overrides, but may only name a shipped adapter; the command form is global only | the `blueprint` module |
| `runner.notify` | global only | the runner |
| `runner.runtime` (compiled adapter name, no default; initially `t3`) | global only | runner |
| `runtimes.<name>` (agent-action instruction values) | global only; complete value set below | configuration and the agent consuming the rendered instruction |
| `runner.autostart` (boolean or map `{<kind>: bool}`) | both; the project overrides | runner |
| `project` | project only | docs and publication |
| `requires` (modules the flavor needs) | project only | publication |
| `board` (GitHub project `<owner>/<number>`; automatically set up and repaired with views) | project only | the `todos` module |
| `github.apps` (App/bot public identities and protected private-key references), `github.project_app` (fallback App reference) | global only | work GitHub boundary; configuration stores and validates declarations |
| `github.approvers` (GitHub logins) | project only | work |
| `github.repository` (`owner/repo`; default derived from the remote the default branch tracks) | project only | work, runner |
| `github.api_url` (default `https://api.github.com`) | global only | work, workflow, runner |
| `approvals.prd`, `approvals.task` (`human` or `peer`, each default `human`) | project only | workflow |
| `workflow.domain` (published domain name, default `sdlc`; a domain published from some project's `docs/domain/<x>/`) | project only | workflow |

Runtime choice is global-only: a cloned repository does not decide which runtime
executes on the machine. Procedures run checks as the project documents them;
there is no project-layer `actions` key. CI required checks remain the merge gate.

### Event configuration values and cross-computer agreement

`redis.cluster.url` is required for enabled transport and identifies one shared
logical database/stream namespace, not a sharded Redis Cluster client. Local URL
is Unix-socket-only; absence uses cluster visibly. `max_connections` and
`socket_timeout` follow the port's connection settings, with timeout outlasting
the blocking read. Credentials remain references and expanded values are masked
in reads and findings. An unavailable configured local endpoint is not absence.

`ingress.computer` is the existing stable instrukt machine ID. All participating
machine global layers agree on that ID and the logical cluster Redis/namespace;
URL spellings and ACL credentials may differ when they reach the same logical
store with required permissions. `ingress.port` is a valid TCP port shared only
by the ingress receiver and local edge. The bind host is fixed loopback. Local
Redis socket paths, local credentials, runtime and ticks settings stay local.
No shared configuration layer or new project-owner-machine routing key exists.

Project identities and checkouts needed for activation must exist on the emitting
computer, including ingress. Signing values needed on ingress resolve from the
one owner-only machine-local `~/.instrukt/secrets.env`. Source secret fields
are exact raw `${NAME}` references; the same file also supplies Redis and all
other instrukt secrets on every entry path. Project `.env` and process
environment supply no fallback. The protected read/write contract is
`project/instrukt/spec/feature/configuration/owned-files`. A declared previous secret follows the same
rules as current. Configuration reads preserve references and never expand
source signing material. Source edits apply at the next request. The domain key selects
`/hooks/inbound/project/<slug>/<domain>` and the emitted type prefix; it is
never taken from a body field. Sources use native `{name, payload}`, a payload
field through `name_from`, fixed `name`, or GitHub normalization through
`verify: github`, as defined by the inbound framework.

Invalid domains, unknown verification schemes, conflicting name sources, misplaced keys, malformed references, unreadable ingress
values and configured sources with effective events off are findings. Unknown
keys remain findings; no direction or `sources` subkey is accepted. The typed
configuration also generates editor schemas. Schema validation itself starts
no receiver, Redis or scheduler; infrastructure convergence belongs to
`project/instrukt/spec/feature/runner/module-installation`.

### Enablement and installation demand

Installation evaluates effective settings for every adopted project on the
computer. Project overrides apply independently; global module defaults without
an adopted enabled project install nothing. `ticks: on` permits the shared minute
wake-up only while todos or events needs it. Todos alone installs no Redis or
receiver. Events with no sources still needs scoped transport but no receiver.
A nonempty valid source map adds receiver demand only on `ingress.computer`.
An unreadable project's configuration never counts as proof its shared resources
are unused.

A revision-checked patch records desired enablement, never successful setup.
The configuration agent follows it with `instrukt install` to converge dependencies
and `instrukt install --check` to verify them. Disable stops workload admission;
explicit convergence removes only owned units with no remaining enabled demand,
retaining data and foreign services. Disabling events with sources declared
requires removing those source declarations in the same valid patch. No new
installer toggle, shared configuration layer or runtime detection is added.

### Runtime choice

The developer chooses global `runner.runtime` by compiled adapter name. `t3`
selects the shipped T3 implementation; an unknown name is a validation finding.
An unset value selects no runtime, even when T3 is installed or a caller is
inside a T3 session. There is no first-entry default or detection.

`runtimes` is a map keyed by runtime name. Each entry holds the named nonempty
agent-action instruction values below. `runner.runtime` selects that entry;
`runtime.<name>` resolves its value at read time. Names and inputs are
runtime-neutral. The entry supplies either its native instruction or an instrukt
route for a capability with no native counterpart. The consuming doc, action,
template or skill references only `@config: runtime.<name>`; it embeds neither
native tool calls nor fallback commands. Instrukt executes none of this prose.
No command/agent verb kind or agent-action exit-status router exists.

### Agent-action values

| Value under `runtimes.<runtime>` | What its instruction conveys |
| --- | --- |
| `dispatch` | Delegate this agent's worker using the supplied brief, role, attempt, semantic key, display title and preferences with shared isolation in the parent's recorded workspace. Return native child identity and matching workspace/branch/head evidence; retain native parent linkage. |
| `message_child` | Send a bounded follow-up, findings, review continuation or recovery to this parent's identified child, retaining its role/attempt and using the native continuation behavior when idle. |
| `message_peer` | Send the supplied substantive text to an addressed peer/coordinator conversation, preserving the target and source attribution. |
| `read_thread` | Read an identified conversation's relevant messages and attribution; state extent, pagination and unavailable evidence. This is not history search. |
| `inspect_thread` | Inspect an identified thread or own child/task's state/result, distinguishing unavailable observation, input wait, failure and confirmed absence. |
| `look_around` | Discover current conversations in the supplied project/scope, including native current-thread metadata search and settled threads where supported. It releases no archive/history search service. |
| `steer` | Deliver current direction at the target's supported safe input boundary while its turn and children continue; state queued/entry degradation when native interjection is absent. |
| `discuss` | Open, participate in and close a runner-owned conversation link between two peer threads; the person participates in either thread and the runner relays finished turns with attribution. |
| `start_thread` | Start a separately addressed conversation with its supplied project, semantic key, display title, initialized brief and placement. A work orchestrator uses `<slug>/work` after preparation integration; preparation uses its separate `<slug>/prepare` worktree. A worker uses `dispatch`. |
| `cancel_child` | Stop this parent's identified child under its existing mandate, retaining its work/evidence and distinguishing accepted cancellation from completion. |
| `finish_thread` | Settle an addressed conversation or this agent's own conversation where the runtime supports it; retain open obligations and apply self-settling at its supported turn boundary. |
| `discover_environments` | Discover connected computers/environments, their project locations, access and task capabilities; keep runtime addresses distinct from instrukt machine IDs. |
| `connect_environment` | Request the runtime's native connection/authentication flow for a named missing computer; report unresolved access as a prerequisite without inventing an address. |
| `dispatch_remote` | Assign a local brief and report destination to an explicitly addressed connected environment, under its project and permissions; follow native completion evidence and retain partial results. |

Each entry supplies the complete set, including a clear unsupported/prerequisite
instruction where neither a native counterpart nor an implemented instrukt route
exists. An unsupported instruction grants no capability. Unknown names, empty
values and project-layer entries are findings. A missing selected entry or value
fails `@config` lookup visibly. Native-capability availability and unattended
adapter availability remain distinct typed facts; no code infers them from prose.
All agent-action instructions, including fallback routes and runtime-specific
worktree constraints, are configuration data. The shipped entry is a data
resource parsed by configuration, not Rust string constants or tool-selection
branches. Changing one of these values changes what a future read renders;
it never reconfigures an existing initialized standing assignment.

### Shipped T3 entry values

The following table specifies the instruction text carried by the `t3` entry.
Only the configuration entry carries the runtime-specific execution wording;
agent-facing consumers retrieve the value through its alias.

| Value | T3 entry instruction |
| --- | --- |
| `dispatch` | Use native delegated-task dispatch with shared isolation from this conversation, preserving its recorded workspace. Each phase has its own recorded worktree: preparation shares <slug>/prepare, while the later work phase shares <slug>/work. All children inherit their orchestrator's phase workspace. Use the supplied brief, semantic key, title and role-attempt facts. Include the role/attempt identity in the child brief and require it to await confirmed instrukt binding before workspace changes, record writes or stamps. Retain the native child/task id and matching workspace/branch/head evidence for instrukt binding. Follow native completion delivery. |
| `message_child` | Use native child/task messaging or continuation for the supplied child id. Relay the bounded follow-up and retain the same role/attempt. Continue an idle child through the native continuation operation. |
| `message_peer` | Use native thread messaging for the supplied peer/coordinator thread id and text. Preserve its identity and the message's attribution. |
| `read_thread` | Read the identified thread with native thread-read operations, including the relevant message range and attribution. Report pagination or unavailable evidence; read no archive/history service. |
| `inspect_thread` | Read the identified native thread or own delegated task through its native status/result operation. Follow native completion/wake instructions; inspect a running task only when those instructions permit it. |
| `look_around` | Use native project thread listing and available current-thread metadata filters. Select the requested project and include settled threads when needed. Retain the runtime's visibility limits for children; listing absence proves no missing child. |
| `steer` | Use native thread/child input with its supported steering mode. Preserve the running turn and children. For a target that queues rather than interjects, hand over now for its next supported turn. |
| `discuss` | When asked to open a discussion, use `instrukt discuss open <thread> [--message <text>]`; the native caller is the other member. Use `instrukt discuss close [--message <text>]` to deliver an optional final contribution and close for both members. The running instrukt runner shares finished turns automatically under project/instrukt/spec/feature/runner/conversation-links. |
| `start_thread` | Use supplied placement.selection profile/kind/instance/model/effort/options for native startup where supported. The host CLI passes --provider and --model and records its effort/options flag gap. For a preparation orchestrator, start t3 thread start <project> - --worktree origin/main --branch <slug>/prepare --title <display-title>. After reviewed docs integrate to main, the integrator removes that worktree and deletes the prepare branch locally and on origin. Start work later from updated main with --worktree origin/main --branch <slug>/work; on another computer use --worktree origin/<slug>/work --branch <slug>/work. instrukt derives these names; creation refuses an existing local branch. Native shared children inherit the phase worktree. After deliberate native startup, enter todo prepare/work with --placement-evidence containing native session, key, title, recorded worktree, branch, head and source; shell cwd alone is not placement evidence. Wake a settled retained orchestrator in place. The CLI cannot attach an existing worktree to a replacement thread; resolve that placement or use native MCP existing-worktree start. Ad hoc work uses its own one-off worktree; only the integrator writes main. |
| `cancel_child` | Use native delegated-task cancellation for the supplied own-child task id, preserving its branch, workspace and unfinished evidence. Read the returned cancellation state before deciding its remaining work. |
| `finish_thread` | Preserve unfinished obligations. Automation never settles a standing agent; it remains unsettled in the sidebar. For other threads use native settling, which applies when its current turn ends. A human-settled standing thread stays reachable and wakes on a message. |
| `discover_environments` | Use the connection's native linked-environment discovery. Match participating computers to observed environment ids and registered project paths, keeping instrukt machine IDs distinct. Report missing mappings and access. |
| `connect_environment` | Use native environment linking with the named computer as its hint and follow the connection's authentication flow. Report any unresolved operator prerequisite. |
| `dispatch_remote` | Use the connection's native task operation explicitly targeting the identified linked environment and registered project. Supply the local configuration brief, skill package location/text and report destination; preserve local permission limits. Follow its completion instructions and compare returned verification across computers, retaining partial success. |

Selection and entry values stay global; a project cannot replace either. A
shipped entry never chooses the runtime on the developer's behalf. T3's host CLI
continues serving unattended adapter starts independently of these agent values;
caller-identity recognition remains an environment-only code operation.

Feature enablement is checked against the selected runtime's native/adapter capabilities
(`project/instrukt/spec/feature/runner/session-continuity`). Without a selected
runtime, enabling runtime-dependent features is refused with that reason. A
missing capability refuses and names the runtime and capability. Configuration
reads and documentation remain usable without a runtime.

### Agent preferences and availability

Model selection is role-keyed configuration in `~/.instrukt/agents.yaml`,
independent of these layers and of procedure content. A procedure's `role`
selects a role mapping, falling back to `default`; per-task overrides win.
The full profile, tier-chain, readiness, availability and upgrade contracts are
`project/instrukt/spec/feature/configuration/agent-selection`. That same role map
carries `class`, scoped `can` grants with named `bundles`, and `github_app` under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. The
machine-global App inventory and fallback identity follow
`project/instrukt/spec/feature/work/github-apps`; keys resolve only through the
protected secrets file. Neither model preferences nor project configuration
selects a bot actor.

### Locating the layers

- Installed instrukt uses `~/.instrukt/instrukt.yaml` for the global layer, including
  in ordinary project worktrees. The repository development wrapper uses its
  isolated home under `project/instrukt/spec/feature/configuration/owned-files`.
- The project layer is the `instrukt.yaml` at the root of the repository
  containing the working directory. Outside a repository, or in one without
  that file, there is no project layer.

### Effective configuration

A read without `--global` answers from the effective configuration: the
global layer with the project layer's keys applied over it, key by key. A read
with `--global` answers from the global layer alone. Writes address one layer
(`project/instrukt/spec/feature/configuration/configuration-editing`).

### Validation

`instrukt config validate` checks the project layer, or the global layer with
`--global`. Findings:

- a key in a layer that does not permit it, naming the key and its layer;
- a value of the wrong type or outside its allowed values;
- a missing `schema_version`;
- a feature switched on whose required capabilities the chosen adapter lacks,
  or switched on with no runtime chosen, naming each missing capability and
  the selected runtime
  (`project/instrukt/spec/feature/runner/session-continuity`). A write
  that would switch such a feature on is refused with the same naming;
- in a project, `modules.todos` on in the effective configuration while the
  project's code is not on GitHub, saying in plain words that todos need the
  project's code on GitHub
  (`project/instrukt/spec/feature/work/github-record`). A write to the
  project layer that would switch todos on there is refused with the same
  reason.

Every finding names the document and key; the exit status is non-zero when
there is at least one finding. Validation judges the document as written;
references are checked for unresolved keys and cycles without executing commands.

### Schema versions and migration

- Every document carries `schema_version`; `init` writes the current one.
- Every configuration path runs migration before schema refusal or skip: a
  document of an older schema version is rewritten in the current version
  with its settings preserved, through the same atomic write as a patch.
  This includes reads, patches, validation, installation, ticks and unchanged
  publication. Other owned files gain versions when their shapes first change
  (`project/instrukt/spec/feature/configuration/owned-files`).
- A document already at the current version is left untouched.

### Environment

`INSTRUKT_NOW=<RFC 3339>` overrides the clock, for verification and
diagnosis. While it is set, every command takes it as the current time in
place of the system clock — for retention windows, schedules and due times,
and the times it records. It is read once, where the command constructs its
clock. It is never silent: while it is set, every command prints
`instrukt: clock overridden to <value>` to stderr. A value that is not RFC 3339
refuses the command; it never falls back to the system clock.

### External state declarations

External names default to occurrence. A project `webhooks.<domain>` source may
set `state: [<name>, …]` and `subject_from: <dotted-payload-field>`. Names omit
the domain prefix and follow the ordinary event-name grammar. State names
require a nonempty subject field declaration. After signature verification,
the field is read from the normalized payload and must be a nonempty string
or number. Its UTF-8 percent-encoded value qualifies the source's ordinary
`inbound_source:<domain>:project:<slug>` subject. Missing or invalid values
produce a client error with no relay. Other names remain occurrences. Ingress
records `inbound_delivery: state` as producer metadata independently of payload
text. Pending-state replacement is owned by procedure-event-delivery.

## Use cases

### UC-CL1: A developer overrides a machine default for one project

```gherkin
Given a developer's machine sets a default for a setting that projects may override
And one of their projects sets its own value for that setting
When an agent acting for the developer reads the setting inside that project
Then it receives the project's value
When it reads the setting inside another project that sets nothing
Then it receives the machine's default
```

### UC-CL2: A developer clones a project that tries to decide what runs on their machine

```gherkin
Given a developer clones a project whose configuration chooses the machine runtime
When the developer validates the project's configuration
Then the declaration is reported as not permitted in a project
And reading the runtime choice inside that project still answers the machine's own
```

### UC-CL3: A developer keeps working after upgrading instrukt

```gherkin
Given a developer's machine and project configuration were written by an older instrukt
When they upgrade instrukt and read configuration or run a scheduled tick
Then their configuration is brought to the current schema with every setting kept
And the commands that read it work as before
```

### UC-CL4: An older instrukt refuses configuration written by a newer one

```gherkin
Given a developer's configuration was written by a newer instrukt than the one they run
When they read, change or validate that configuration
Then each command refuses and tells them to upgrade instrukt
And the configuration is unchanged
```

### Configuration value shapes

| Key | Type / allowed values | Default |
| --- | --- | --- |
| `ticks` | `on` or `off` | `on` (installation registers one wake-up only with enabled project demand; `off` selects operator scheduling). |
| `runner.runtime` | compiled runtime adapter name (`t3`) | absent; no runtime is chosen |
| `runner.notify` | command string or `off` | the selected runtime’s notification capability when unset |
| `runner.autostart` | boolean or map of kind to boolean | `false` (manual); in a map, an omitted kind is `false`. |
| `agents.<name>.git_author` | git author identity string (name and email) | absent; an agent commit without it is refused |
| `board` | GitHub project reference `<owner>/<number>` | automatically set up and repaired with its views when todos is enabled; absence triggers setup rather than opt-out. |
| `blueprint.engine` | shipped adapter name (either layer) or command string (global layer only; in the project layer a finding, ignored like project-layer `runner.runtime`) | the shipped adapter |
| `schema_version` | schema version number written by init | current schema version; the first is `1`. |

### UC-CL5: Configure external state in its owning source

```gherkin
Given a developer configures external state names for one project source
When their declaration supplies the subject field and valid names
Then configuration retains that source declaration
But a state declaration without its subject field is refused with a finding
```

### UC-CL6: Obtain agent actions from the chosen runtime's configuration

```gherkin
Given a developer chose T3 and customized its agent-action values globally
When an agent retrieves a procedure using those instructions
Then its dispatch, child follow-up, peer message, conversation read, status inspection, listing and steering instructions come from that entry
And instrukt executes none of that prose
But a cloned project cannot replace the machine's agent-action values
```

## Known caveats

- An unknown key is a validation finding. There is no `publishes` key: what a
  project publishes follows from its `docs/general/` and `docs/domain/<x>/`
  positions, and a leftover `publishes` is an unknown key, never migrated.
- The project layer carries the `yaml-language-server` modeline naming its
  editor schema; it is a comment and never a setting
  (`project/instrukt/spec/feature/publication/editor-schemas`).
- Every entry path migrates older configuration before refusal or skip. All
  commands refuse a newer schema version and direct the caller to upgrade instrukt.
- Unresolved and cyclic references are validation findings.
- `requires` naming a switched-off module is a validation finding.
- Runtime capability metadata belongs to the shipped integration; adapter
  behavior requires code. Agent instruction prose remains configuration.
  Arbitrary executable runtime command templates are not supported.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](configuration-editing.md)
- [project/instrukt/spec/feature/configuration/interpolation](interpolation.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
