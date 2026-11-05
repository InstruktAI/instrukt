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

Runtime entries may declare an `available` check template. `dispatch` agent names validate against agents the configured runtime mapping can start, never a fixed set.

### Layer rules

| Key | Layer | Meaning owned by |
| --- | --- | --- |
| `schema_version` | each document | configuration |
| `runtimes` (runtime mapping) | global only | configuration (selection); readers of each entry |
| `agents.order`, `agents.thinking_mode` (ordered agent preferences and their thinking mode) | global only | dispatch |
| `agents.<name>.git_author` | global only | commit composition |
| `modules: {blueprint, todos, events}` (booleans, default off; core has no switch) | both (project overrides global) | each module |
| `events.watch: [{remote, ref}]` | project only | runner |
| `ticks` | global only | the runner |
| `blueprint.engine` | both; the project overrides, but may only name a shipped adapter; the command form is global only | the `blueprint` module |
| `runner.notify` | global only | the runner |
| `runner.runtime` (the chosen runtime: an entry of `runtimes` or a shipped entry; default: first entry of `runtimes`) | global only | runner |
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

A runtime mapping is never taken from a project: a cloned repository must not
decide what runs on a machine, as git ignores machine-level keys in repository
configuration. `actions` are called by name from procedures and never run by
hooks.

### Runtime choice and shipped entries

The developer chooses the runtime; no configuration default detects or picks
one. instrukt ships runtime entries the developer may choose by name; today
that is `t3`, for T3 Code, filled from T3's `t3 thread` commands
(`project/instrukt/spec/feature/runner/session-continuity`).

- `runner.runtime: <name>` chooses a runtime. It names an entry of `runtimes`
  or a shipped entry; any other name is a validation finding.
- A chosen shipped entry joins the effective `runtimes` exactly as shipped,
  after the declared entries. A declared entry with the same name replaces it.
  A shipped entry nobody chose is not in the effective mapping.
- With `runner.runtime` unset, the runtime is the first declared entry of
  `runtimes`; with neither set there is no runtime.
- `config get runtimes` prints the effective mapping, shipped entries
  included, as stored.

Shipping an entry adds configuration only: the runner drives it through the
same verbs as a declared entry.

### Runtime entries

Each entry of `runtimes` is a map:

| Key | Value |
| --- | --- |
| `name` | the entry's name, unique in the mapping |
| `session_id_env` | the environment variable that holds the caller's session id inside this runtime |
| `agent` | optional: the agent CLI this entry is (`claude`, `codex`, …); its git author comes from `agents.<name>.git_author` |
| `start`, `send`, `alive`, `conversation`, `notify`, `available`, `model` | optional verbs of kind `command` |
| `end`, `end_self`, `link` | optional verbs of kind `command` or `agent` |
| `history` | optional map with the verbs `search` and `show`, each of kind `command` or `agent` |

A verb's value has one of these forms:

| Form | Kind |
| --- | --- |
| a string | `command`: the string is the command template |
| `{kind: command, template: <string>}` | `command` |
| `{kind: agent, instruction: <string>}` | `agent`: the instruction the calling agent carries out with its own runtime's tools |

An absent verb is empty. A map with another `kind`, with both `template` and
`instruction`, or with the field its kind does not take, is a validation
finding; so is a verb of kind `agent` where only `command` is allowed. What
each verb does and how `instrukt sessions` treats each kind is
`project/instrukt/spec/feature/runner/session-continuity`.

```yaml
runtimes:
  - name: host
    session_id_env: HOST_SESSION_ID
    start: host start
    send: host send {runtime.session_id}
    alive: host alive {runtime.session_id}
    conversation: host show {runtime.session_id}
    end:
      kind: agent
      instruction: Close session {runtime.session_id} with your host's close tool.
    history:
      show: host show
```

`instrukt config get runtime.<verb>` and a `<!-- @config: runtime.<verb> -->`
directive give the active runtime's verb as its expanded template, or its
expanded instruction for kind `agent`
(`project/instrukt/spec/feature/configuration/interpolation`).

### Agent preferences and availability

The global layer supplies a non-empty ordered agent preference list,
`agents.order: [<agent>, …]`, and one thinking mode for the whole list,
`agents.thinking_mode` (`fast`, `med` or `slow`) — the same shape as a
procedure's `dispatch` (`agent: [<agent>, …]`, `thinking_mode`). Precedence,
first to last: a task's own preference (`--agent <name> [--thinking …]`, one
agent with its own mode), then the procedure's `dispatch`, then
`agents.order` with `agents.thinking_mode`. `agents.thinking_mode` has no
default: it is required whenever `agents.order` is set, and a missing mode is a
validation finding, as for `dispatch`. Agent names validate against the
agents the configured runtime mapping can start, rather than a fixed set or
the shorter set of agents receiving deployed instructions.

A runtime entry may declare an `available` check template. Configuration supplies
the preferences and optional check; dispatch execution belongs to
`project/instrukt/spec/feature/runner/work-dispatch`. Composing the order
is agent judgment informed by `general/concept/agent-characteristics`.

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
- a feature switched on whose verbs the chosen runtime does not fill, or
  switched on with no runtime chosen, naming each missing verb and the
  runtime entry
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
Given a developer clones a project whose configuration declares which runtimes the machine runs
When the developer validates the project's configuration
Then the declaration is reported as not permitted in a project
And reading the runtimes inside that project still answers the machine's own
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
| `ticks` | `on` or `off` | `on` (the install registers the wake-up; `off` opts out). |
| `runner.runtime` | name of a `runtimes` entry or a shipped entry (`t3`) | the first entry of `runtimes`; with none, no runtime |
| `runner.notify` | command string or `off` | the selected runtime’s notification capability when unset |
| `runner.autostart` | boolean or map of kind to boolean | `false` (manual); in a map, an omitted kind is `false`. |
| `agents.<name>.git_author` | git author identity string (name and email) | absent; an agent commit without it is refused |
| `board` | `false` or `<owner>/<number>`; `true` is a finding asking for the project reference | `false` (todos are always issues; the board is opt-in). |
| `blueprint.engine` | shipped adapter name (either layer) or command string (global layer only; in the project layer a finding, ignored like project-layer `runtimes`) | the shipped adapter |
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
- Runtime entries carry `name` and `session_id_env`, and may fill the verbs `start`, `send`, `alive`, `conversation`, `notify`, `link`, `end`, `end_self`, `available`, `model`, `history.search` and `history.show`, each of kind `command` or `agent` (`Runtime entries`); an empty verb leaves the features that need it unavailable. `agent: <name>` marks an agent CLI; its git author comes from `agents.<name>.git_author`.

## See Also

- project/instrukt/spec/domain/configuration
- project/instrukt/spec/feature/configuration/configuration-editing
- project/instrukt/spec/feature/configuration/interpolation
