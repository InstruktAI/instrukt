---
description: "`instrukt.yaml` in a global and a project layer — which key lives where, how the layers combine, validation, schema versions and migration."
delivered_by: [instrukt-first-build]
---

# Layered Configuration — Spec

## What it is

instrukt has one configuration file name and one schema, read in two layers,
like git's user and repository configuration. The global layer
(`~/.instrukt/instrukt.yaml`) holds a machine's defaults and what only the
machine may decide; the project layer (`instrukt.yaml` at the repository root)
holds what the project declares, and overrides the global layer where a key
may live in both. Validation knows, per key, which layer it may live in. Both
layers carry `schema_version`, and older documents are migrated.

## Canonical fields

`runner.runtime` chooses a compiled adapter. Dispatch preferences are checked
against the selected adapter's declared capabilities, never executable
configuration.

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
| `agents.order`, `agents.thinking_mode` (ordered agent preferences and their thinking mode) | global only | dispatch |
| `agents.<name>.git_author` | global only | commit composition |
| `modules: {blueprint, todos, events}` (booleans, default off; core has no switch) | both (project overrides global) | each module |
| `events.watch: [{remote, ref}]` | project only | runner |
| `webhooks.native`, `webhooks.github` | project only; each takes required `secret` and optional `previous_secret` references | runner intake |
| `ingress.computer`, `ingress.port` | global only; stable existing machine ID and loopback port | runner intake |
| `redis.cluster`, `redis.local` | global only; `url`, optional `username`/`password` references and connection limits/timeouts | runner transport |
| `ticks` | global only | the runner |
| `blueprint.engine` | both; the project overrides, but may only name a shipped adapter; the command form is global only | the `blueprint` module |
| `runner.notify` | global only | the runner |
| `runner.runtime` (compiled adapter name, no default; initially `t3`) | global only | runner |
| `runner.autostart` (boolean or map `{<kind>: bool}`) | both; the project overrides | runner |
| `project` | project only | docs and publication |
| `requires` (modules the flavor needs) | project only | publication |
| `actions` (name → the project's own command) | project only | the flavor's procedures |
| `board` (`false`, or the GitHub project `<owner>/<number>` todos are reflected to) | project only | the `todos` module |
| `github.approvers` (GitHub logins) | project only | work |
| `github.repository` (`owner/repo`; default derived from the remote the default branch tracks) | project only | work, runner |
| `github.api_url` (default `https://api.github.com`) | global only | work, workflow, runner |
| `approvals.prd`, `approvals.task` (`human` or `peer`, each default `human`) | project only | workflow |
| `workflow.domain` (published domain name, default `sdlc`; a domain published from some project's `docs/domain/<x>/`) | project only | workflow |

Runtime choice is global-only: a cloned repository does not decide which runtime
executes on the machine. `actions` are called by name from procedures and never
run by hooks.

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
computer, including ingress. Only ingress needs a declaring project's `.env`
signing values. Source secret fields are exact raw `${NAME}` references;
ingress validates their nonblank values solely in that project's `.env`, with
no process-environment fallback. A declared previous secret follows the same
rules as current. Configuration reads preserve references and never expand
source signing material. Source edits apply at the next request.

Invalid protocols, misplaced keys, malformed references, unreadable ingress
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

There is no `runtimes` list, shipped YAML entry, verb kind, command template,
agent instruction or configurable `session_id_env`. These pre-release keys are
unknown and not migrated. T3 uses the installed host CLI, the registered project
checkout and the project's own model and permission defaults; this adapter
requires no additional configuration keys. Its caller-identity recognition
belongs to code, not configuration.

Feature enablement is checked against the selected adapter's capabilities
(`project/instrukt/spec/feature/runner/session-continuity`). Without a selected
runtime, enabling runtime-dependent features is refused with that reason. A
missing capability refuses and names the runtime and capability. Configuration
reads and documentation remain usable without a runtime.

### Agent preferences and availability

The global layer supplies a non-empty ordered agent preference list,
`agents.order: [<agent>, …]`, and one thinking mode for the whole list,
`agents.thinking_mode` (`fast`, `med` or `slow`) — the same shape as a
procedure's `dispatch` (`agent: [<agent>, …]`, `thinking_mode`). Precedence,
first to last: a task's own preference (`--agent <name> [--thinking …]`, one
agent with its own mode), then the procedure's `dispatch`, then `agents.order`
with `agents.thinking_mode`. `agents.thinking_mode` has no default: it is
required whenever `agents.order` is set, and a missing mode is a validation
finding, as for `dispatch`. Agent names validate against the agents the selected
runtime adapter can start, rather than a fixed set or the shorter set of agents
receiving deployed instructions.

Configuration supplies preferences; the adapter supplies supported selection and
availability metadata. It exposes unsupported selection explicitly; no command
template is configured. Dispatch execution belongs to
`project/instrukt/spec/feature/runner/work-dispatch`. Composing the order is
agent judgment informed by `general/concept/agent-characteristics`.

### Locating the layers

- The global layer is always `~/.instrukt/instrukt.yaml`.
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
- `instrukt config init` and publication run the migration operation: a document
  of an older schema version is rewritten in the current version with its
  settings preserved, through the same atomic write as a patch.
- A document already at the current version is left untouched.

### Environment

`INSTRUKT_NOW=<RFC 3339>` overrides the clock, for verification and
diagnosis. While it is set, every command takes it as the current time in
place of the system clock — for retention windows, schedules and due times,
and the times it records. It is read once, where the command constructs its
clock. It is never silent: while it is set, every command prints
`instrukt: clock overridden to <value>` to stderr. A value that is not RFC 3339
refuses the command; it never falls back to the system clock.

### Use cases

#### UC-CL1: A developer overrides a machine default for one project

```gherkin
Given a developer's machine sets a default for a setting that projects may override
And one of their projects sets its own value for that setting
When an agent acting for the developer reads the setting inside that project
Then it receives the project's value
When it reads the setting inside another project that sets nothing
Then it receives the machine's default
```

#### UC-CL2: A developer clones a project that tries to decide what runs on their machine

```gherkin
Given a developer clones a project whose configuration chooses the machine runtime
When the developer validates the project's configuration
Then the declaration is reported as not permitted in a project
And reading the runtime choice inside that project still answers the machine's own
```

#### UC-CL3: A developer keeps working after upgrading instrukt

```gherkin
Given a developer's machine and project configuration were written by an older instrukt
When they upgrade instrukt and initialize or publish as usual
Then their configuration is brought to the current schema with every setting kept
And the commands that read it work as before
```

#### UC-CL4: An older instrukt refuses configuration written by a newer one

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
| `board` | `false` or `<owner>/<number>`; `true` is a finding asking for the project reference | `false` (todos are always issues; the board is opt-in). |
| `blueprint.engine` | shipped adapter name (either layer) or command string (global layer only; in the project layer a finding, ignored like project-layer `runner.runtime`) | the shipped adapter |
| `schema_version` | schema version number written by init | current schema version; the first is `1`. |

An action is a command string, which takes no files, or a declaration
`{command: <string>, takes_files: true}`; `takes_files` defaults to `false`.
For a file-taking action, procedures append file paths as arguments to the
command read with `config get actions.<name>`; no action-runner command is
introduced.


## Known caveats

- An unknown key is a validation finding. There is no `publishes` key: what a
  project publishes follows from its `docs/general/` and `docs/domain/<x>/`
  positions, and a leftover `publishes` is an unknown key, never migrated.
- The project layer carries the `yaml-language-server` modeline naming its
  editor schema; it is a comment and never a setting
  (`project/instrukt/spec/feature/publication/editor-schemas`).
- `init` and `sync` migrate older schema versions in place; other commands refuse and name that door. All commands refuse a newer schema version and direct the caller to upgrade instrukt.
- Unresolved and cyclic references are validation findings.
- `requires` naming a switched-off module is a validation finding.
- Runtime capabilities are compiled adapter behavior. Future adapters require code; arbitrary runtime command templates are not supported.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](configuration-editing.md)
- [project/instrukt/spec/feature/configuration/interpolation](interpolation.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
