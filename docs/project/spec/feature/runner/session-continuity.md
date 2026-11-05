---
description: Runtime-mapped session operations, the runtime boundary, template-gated features and the shipped T3 Code entry, agent-facing history templates, recorded-session reuse, five-state observation and record-backed step completion.
visibility: public
delivered_by: [instrukt-first-build]
---
# Session Continuity — Spec

## What it is

The sessions facade lets an agent continue a person's work through the chosen
runtime while preserving the recorded conversation. Runtime-specific commands
stay in the machine's runtime mapping.

The runtime owns sessions. Session lifecycle, input delivery, restart,
revival, handover, transcripts, cleanup and subscriptions to session activity
belong to the runtime; instrukt builds none of them. Events, schedules and
dispatch are instrukt's own. instrukt calls the runtime only through the
command templates of the runtime mapping, and records the session ids those
calls return. A feature is available only when the chosen runtime fills the
templates it needs. instrukt ships a mapping entry for T3 Code, which a
developer chooses as their runtime.

## Canonical fields

### Public surface

| Command | Contract |
| --- | --- |
| `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file|->] [--worktree <name>] [--todo <slug>]` | Start a session through the runtime mapping and obtain its runtime-native id. With `--todo`, record the session as that todo's binding for the started procedure's `role`; without it, nothing is bound. |
| `instrukt sessions send <id> <text>` | Send work to an identified session through that mapping. |
| `instrukt sessions alive <id>` | Observe whether the recorded session remains available for reuse and its status. |
| `instrukt sessions link <a> <b>` | Link sessions when the runtime entry declares a `link` template; otherwise report that the runtime lacks the capability. |
| `instrukt sessions end <id>` | End the identified session through the optional `end` template; otherwise report that the runtime lacks the capability. |
| `instrukt sessions link --remove <a> <b>` | Remove the link through the runtime mapping; report a missing capability when unsupported. |
| `instrukt sessions resolve-start <procedure>@<project> --session <runtime:id> \| --none` | Resolve a standing agent's start whose result never arrived: bind the session the runtime did create, or declare that none was, allowing a fresh start. |
| `instrukt sessions conversation <id>` | Read the identified session's conversation through that mapping. |

Runtime command templates receive per-call input as follows:

- Per-call inputs (the opening message, sent text, another session's id) are
  appended to the command as separate arguments, never spliced into the
  command text; their content may be anything.
- `start` also receives `INSTRUKT_WORKTREE`, `INSTRUKT_AGENT` and
  `INSTRUKT_THINKING` in its environment.
- `start` prints the session id, then the worktree path when one was
  requested; no path means the mapping lacks worktree support.
- The opening message names the procedure's closure read (``Run `instrukt docs
  get <procedure> --closure` …``), then the bound parameters, then, for a task
  run, `occurrence: <RFC 3339>` naming the scheduled occurrence, then the brief.

The mapping is global configuration, not executable instructions supplied by
a cloned project's configuration. Configuration interpolation expands command
templates at call time. The active runtime inside an agent is the first
configured entry whose `session_id_env` is set. A containing runtime precedes
the agent runtime it hosts when both identify the caller. With no matching
entry, a caller-relative `runtime.*` lookup fails clearly.

An OS-started runner uses the runtime the developer chose: global
`runner.runtime`, naming an entry of `runtimes` or an entry instrukt ships,
and defaulting to the first entry in `runtimes`. No code detects or chooses a
runtime: with neither set there is no runtime, and every feature that needs
one is unavailable. Each runtime entry declares `name` and `session_id_env`,
and fills any of the templates `start`, `send`, `alive`, `conversation`,
`notify`, `link`, `end`, `available` and `model`, and the agent-facing
`history.search` and `history.show`. An entry with `agent: <name>` is an
agent CLI; its git author comes from `agents.<name>.git_author`.
One runner per computer uses its own runtime; targeting another computer's
sessions is outside this contract.

Every runtime is reached through its mapping entry's templates alone. instrukt
has no code path for a particular runtime: an entry instrukt ships is driven
by the same facade, through the same templates, as one a developer maps
themselves.

### Template-gated features

A feature is available only when the runtime the developer chose fills every
template it needs. Switching a feature on otherwise is refused, naming each
missing template and the runtime entry it is missing from; the setting is left
unchanged. `instrukt config validate` reports an enabled feature whose
templates the chosen runtime does not fill, with the same naming.

| Feature | Switched on by | Templates it needs |
| --- | --- | --- |
| Standing agents | `modules.events: true` | `start`, `send`, `conversation`, `alive` |
| Todo dispatch | `modules.todos: true` | `start` with worktree support, `send`, `conversation`, `alive` |
| Recurring tasks | `modules.todos: true` | `start`, `conversation`, `alive` |
| Anomaly notification | `runner.notify` unset (the runtime's notification) | `notify` |
| Session status | always on; `instrukt sessions alive` | `alive` |
| Linked-session input | always on; `instrukt sessions link` | `link` |
| Ending a session | always on; `instrukt sessions end` | `end` |
| History lookup by agents | always on; `runtime.history.search`, `runtime.history.show` | `history.search`, `history.show` respectively |

Standing agents, todo dispatch and recurring tasks observe their sessions'
status for reuse and completion, so they need `alive` as well as the read.
An always-on command whose template is missing refuses with the missing
capability, as `link` and `end` already do. `runner.notify` set to a command
makes anomaly notification available without the runtime's `notify`.

### Runtime boundary

| Concern | Owner |
| --- | --- |
| Session lifecycle: creating, running, restarting and reviving a session, and handing it over between devices or providers | the runtime |
| Input delivery: queueing a message for a busy session and delivering it as its next turn | the runtime |
| Transcripts and conversation history: storing, paging, importing and searching what was said | the runtime |
| Cleanup of what a session owns: its processes, terminals, previews and attachments | the runtime |
| Subscriptions to session activity | the runtime |
| Events, schedules and dispatch decisions: the OS scheduler wakes the runner, which reads state, logs events, computes schedules and decides dispatch | instrukt; a runtime's own task, schedule or event features are not used |
| Which session holds which work: per-role and standing-agent session bindings | instrukt, recorded by work and runner |
| When a session is started, sent work or ended, and what its opening message carries | instrukt, through the facade |
| Whether a step is complete | instrukt, from the work record's artifact and stamp |

instrukt calls the runtime only through the mapping's templates. A template
the chosen runtime does not fill leaves its features unavailable
(`Template-gated features`); the capability is never rebuilt in instrukt.

### Agent-facing history templates

`history.search` and `history.show` are templates an agent carries out
itself; the runner never runs them. An agent reads them with
`instrukt config get runtime.history.search` and `runtime.history.show`, or
through a snippet's `<!-- @config: runtime.history.search -->` directive,
resolved for the runtime the agent is running in
(`project/instrukt/spec/feature/configuration/interpolation`).

- `history.search` finds earlier conversations by text and names their
  session ids; the agent supplies the text.
- `history.show` shows one conversation's messages; the agent supplies the
  session id, as a search named it or a record carries it.

Each value is a command the agent runs with its input appended, filled from
that runtime's own search and show commands. Doctrine never names a runtime's
command or tool for history; it names these two paths. A runtime entry that
leaves one empty makes its lookup a refusal naming the missing template.
`instrukt sessions conversation <id>` stays the facade's read of a recorded
session through the entry's `conversation` template.

### Shipped T3 Code entry

instrukt ships one runtime entry, `t3`, for T3 Code. A developer chooses it
with `runner.runtime: t3`; nothing selects it for them. Chosen, it is part of
the effective `runtimes` as shipped, after any declared entries, unless the
global layer declares an entry named `t3`, which then replaces it
(`project/instrukt/spec/feature/configuration/layered-configuration`).

The entry is filled from T3's host command `t3 thread`, whose subcommands are
`start`, `send`, `show` and `list`. They reach the T3 server running on the
same computer without a sign-in.

| Key | Filled with |
| --- | --- |
| `session_id_env` | `T3CODE_THREAD_ID`, which T3 sets in every thread's shells |
| `start` | `t3 thread start .`, run in the project checkout, with the opening message appended. A worktree request adds `--worktree` with the project's default branch as its base and `--branch` with the requested name, and prints the thread's worktree path, read from `t3 thread show --json`, on the line after the id. |
| `send` | `t3 thread send {runtime.session_id}`, with the text appended. A busy thread receives it as its next turn; nothing is interrupted. |
| `alive` | `t3 thread show --json --messages 0 {runtime.session_id}`, translated into the five states (below). |
| `conversation` | `t3 thread show --messages 100 {runtime.session_id}` |
| `history.show` | `t3 thread show --messages 100`, with the session id appended |
| `end`, `notify`, `link`, `history.search`, `available`, `model` | empty: `t3 thread` has no command that ends a thread, raises a notification, links threads, searches conversations, checks a provider, or reports a model and version |

So on T3 standing agents, todo dispatch, recurring tasks and session status
are available. These are unavailable:

- ending a session: `instrukt sessions end` refuses naming `end`; what happens
  to a thread after its work is T3's;
- anomaly notification through the runtime: unavailable unless `runner.notify`
  is set to a command;
- linked-session input;
- history search by agents: `runtime.history.search` refuses naming the
  missing template; `runtime.history.show` is available.

Without an `available` check, a failed start tries the next agent. A thread
runs on the T3 project's default model and permission mode; agent access
follows T3's permission modes.

`alive` translates the thread's status. A failed observation, where the T3
server cannot be reached or its answer cannot be read, yields no state and is
retried next tick.

| T3 thread | Five-state status |
| --- | --- |
| `preparing`, `queued`, `starting`, `running` | `running` |
| `waiting`, or any pending request | `needs-input` |
| `idle`, `completed` | `ended ok` |
| `failed`, `interrupted`, `cancelled`, `rolled_back` | `ended failed` |
| unknown thread id, or archived | `gone` |

### Identity and reuse

The work record keeps the started session id per todo and role. Reuse is a
lookup of that id followed by `alive`; brief similarity is not an identity
mechanism. An available session receives further work through `send`. A
successful ended turn can belong to an available session; ending a turn does
not by itself mean the conversation is gone.

Artifact writers and stamps identify their session as `{runtime}:{id}`. The
runtime-native id is retained; instrukt does not mint a second identity for
the same session. Conversation access supplies evidence to the reader checking
recorded intent; work owns that check and its stamp.

### Five-state status contract

The facade translates runtime observations into these five semantic states.
Runtime execution status and workflow completion are separate observations.

| State | Meaning |
| --- | --- |
| `running` | The requested work is still in progress in the runtime. |
| `needs-input` | Work waits for human input in its session thread. |
| `ended ok` | The observed run ended successfully; this says nothing by itself about its required record writes. |
| `ended failed` | The observed run ended unsuccessfully. |
| `gone` | The identified session is no longer available in the runtime. |

The runtime entry's `alive` template returns one of these five states. A
failed observation is not `gone`; the runner retries observation next tick.
A step is done only when the work record carries its required artifact and
stamp. If execution ended without the required artifact or without the required
stamp, or the session is gone, the step remains incomplete: the runner sends to that same session
once. If the required evidence or session stays missing, it raises an anomaly.

### Linked sessions

The sessions facade exposes linked-session input when the configured runtime
mapping declares that capability. What the human types in one session also
reaches a linked session, identified as that human's words. A receiving agent
can distinguish the human's contribution from a message authored by another
agent. Support is a property of the runtime mapping, not an assumption about
every runtime.

### Runtime worktree binding

Setup-only `instrukt sessions run --worktree <name> --brief <file|->` requests
runtime-owned creation or
reattachment. The session binding retains its returned path after the session
ends; observation of the filesystem determines `worktree.exists`. The path is
exposed from that assignment’s build-role binding as
`worktree.path`; Git effects run there. After creation and upstream synchronization, make the opening commit
and run the project’s `prepare` action when
declared, before implementation.
A mapping without worktree support reports the capability gap. A setup-only
run starts no procedure and binds nothing; the worktree path reaches a binding
through the session started into the build procedure with `--worktree` and
`--todo`.

The todo's session bindings are its assignment child set (`sessions.bound`).
Release with `instrukt todo update <slug> --unclaim`, confirm `claim.held` is
false, then call `instrukt sessions end <id>` for each binding. End the lane
only after its obligations finish; preserve an unfinished human conversation.

`instrukt sessions alive <id>` prints one of the five statuses; `run` and
`send` print the session id they used; every facade command accepts
`--json`. A status that cannot be observed is reported as such, never as
`gone`. A procedure launch with `run` loads the named procedure, binds `--args` according to its declared
parameters, and carries `--brief <file|->` as additional briefing content
(from the file or standard input). These flags also apply when `--worktree`
is supplied. Worktree support is a capability of the runtime start template,
not a separate runtime-entry key.

`--procedure` is required for work dispatch. The sole exception is a setup-only
`run --worktree <name> --brief <file|->`: it creates or reattaches the worktree,
returns and records its path, and waits with no implementation instruction.
Without `--procedure`, `--worktree` and a setup brief are required and
`--args` is refused. Implementation requires a later procedure dispatch after
build-start and checklist evidence exist. `send` takes one text operand; pass
it as one shell-quoted argument or through an argument-vector API, never by
interpolating unescaped prose into shell command text.

### Use cases

#### UC-SS1: Share a clarification with a linked collaborator

```gherkin
  Given a developer is working with linked conversations
  And the configured runtime supports sharing human input between them
  When the developer types a clarification in one conversation
  Then the clarification also reaches the linked conversation
  And is identified there as the developer's words
```

#### UC-SS2: A developer settles a standing agent whose start was interrupted

```gherkin
  Given a developer was told that a standing agent's start was interrupted and its outcome is unknown
  And the runner starts no new session for that agent while the outcome is unknown
  When the developer reports that no session was created
  Then the runner may start that standing agent again on its next wake-up
  But when the developer names the session that was created instead
  Then that session is bound to the standing agent and no second one is started
```

#### UC-SS3: A developer on T3 Code has agents work after choosing T3 as their runtime

```gherkin
  Given a developer runs T3 Code on their computer
  And has chosen T3 Code as their runtime, mapping nothing of their own
  When work is dispatched to an agent in one of their projects
  Then the agent's conversation is started in T3 Code for that project
  And further work for that agent reaches the same conversation
  And the developer's agents can read that conversation through instrukt
```

#### UC-SS4: An agent looks up what an earlier conversation said

```gherkin
  Given a developer's runtime mapping fills how their runtime searches and shows past conversations
  When an agent following doctrine needs what an earlier conversation said about a subject
  Then the doctrine it reads gives it that runtime's search, and the show for a conversation it finds
  But when the runtime it is running in leaves history search empty
  Then it is told which template is missing, and no command is given
```

#### UC-SS5: A developer switches on a feature their runtime cannot carry

```gherkin
  Given a developer's chosen runtime leaves empty a template a feature needs
  When they switch that feature on
  Then the change is refused, naming each missing template and the runtime it is missing from
  And their configuration is unchanged
  But when their runtime fills every template that feature needs
  Then the feature is switched on
```

#### UC-SS6: A developer who has chosen no runtime is never given one

```gherkin
  Given a developer has T3 Code installed but has chosen no runtime
  When they switch on work dispatch or standing agents
  Then the change is refused, saying that no runtime is chosen
  And no runtime is picked for them
```

## Known caveats

- T3 selects a provider only together with a model slug, and instrukt's agent
  names are not model slugs, so the shipped `start` selects none: dispatch's
  agent order does not choose the model of a thread started through the
  shipped entry.

## See Also

- project/instrukt/spec/feature/configuration/layered-configuration — choosing the runtime and the shipped entry
- project/instrukt/spec/feature/configuration/interpolation — the active-runtime view agents read templates through
- project/instrukt/spec/feature/runner/anomaly-notification — `runner.notify` when the runtime offers no notification
