---
description: Runtime-mapped session operations, recorded-session reuse, five-state observation and record-backed step completion.
visibility: public
delivered_by: [instrukt-first-build]
---
# Session Continuity — Spec

## What it is

The sessions facade lets an agent continue a person's work through the chosen
runtime while preserving the recorded conversation. Runtime-specific commands
stay in the machine's runtime mapping.

## Canonical fields

### Public surface

| Command | Contract |
| --- | --- |
| `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file|->] [--worktree <name>]` | Start a session through the runtime mapping and obtain its runtime-native id. |
| `instrukt sessions send <id> <text>` | Send work to an identified session through that mapping. |
| `instrukt sessions alive <id>` | Observe whether the recorded session remains available for reuse and its status. |
| `instrukt sessions link <a> <b>` | Link sessions when the runtime entry declares a `link` template; otherwise report that the runtime lacks the capability. |
| `instrukt sessions end <id>` | End the identified session through the optional `end` template; otherwise report that the runtime lacks the capability. |
| `instrukt sessions link --remove <a> <b>` | Remove the link through the runtime mapping; report a missing capability when unsupported. |
| `instrukt sessions conversation <id>` | Read the identified session's conversation through that mapping. |

The mapping is global configuration, not executable instructions supplied by
a cloned project's configuration. Configuration interpolation expands command
templates at call time. The active runtime inside an agent is the first
configured entry whose `session_id_env` is set. A containing runtime precedes
the agent runtime it hosts when both identify the caller. With no matching
entry, a caller-relative `runtime.*` lookup fails clearly.

An OS-started runner uses global `runner.runtime`, defaulting to the first
entry in `runtimes`. Each global runtime entry declares `name`,
`session_id_env`, and command templates `start`, `send`, `alive`,
`conversation`, with optional `notify`, `link`, `end` and `available`. An entry with `agent: <name>` is an
agent CLI; its git author comes from `agents.<name>.git_author`.
One runner per computer uses its own runtime; targeting another computer's
sessions is outside this contract.

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
A mapping without worktree support reports the capability gap.

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
