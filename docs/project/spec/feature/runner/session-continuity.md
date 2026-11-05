---
description: Runtime-mapped session operations, command and agent verb kinds with the "yours to do" status, the runtime boundary, verb-gated features and the shipped T3 Code entry, optional session ending, agent-facing history verbs, recorded-session reuse, five-state observation and record-backed step completion.
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
dispatch are instrukt's own. instrukt reaches the runtime only through the
verbs of the runtime mapping, and records the session ids those calls return.
Each verb is a `command`, which instrukt runs, or an `agent` instruction,
which the calling agent carries out with its own runtime's tools;
`instrukt sessions <verb>` is the one place that decides which. A feature is
available only when the chosen runtime fills the verbs it needs. instrukt
ships a mapping entry for T3 Code, which a developer chooses as their runtime.
Doctrine names only `instrukt sessions …` and `@config` values, never a
runtime or its tools; runtime-specific text lives in the mapping entry alone.

## Canonical fields

### Public surface

| Command | Contract |
| --- | --- |
| `instrukt sessions run [--procedure <id>] [--args <text>] [--brief <file|->] [--worktree <name>] [--todo <slug>]` | Start a session through the runtime mapping and obtain its runtime-native id. With `--todo`, record the session as that todo's binding for the started procedure's `role`; without it, nothing is bound. |
| `instrukt sessions send <id> <text>` | Send work to an identified session through that mapping. |
| `instrukt sessions alive <id>` | Observe whether the recorded session remains available for reuse and its status. |
| `instrukt sessions link <a> <b>` | Link sessions when the runtime entry fills the `link` verb; otherwise report that the runtime lacks the capability. |
| `instrukt sessions end <id>` | End the identified session through the runtime's `end` verb (`Ending sessions`). |
| `instrukt sessions end --todo <slug>` | End each session bound to the todo in instrukt's own session bindings, one `end` per bound session, except the caller's own. |
| `instrukt sessions end self` | End the caller's own session, identified by the active runtime's `session_id_env`, through `end_self`, or `end` with the caller's id when `end_self` is empty. Always the caller's last step. |
| `instrukt sessions history search <text>` | Find earlier conversations by text through the `history.search` verb. |
| `instrukt sessions history show <id>` | Show one conversation's messages through the `history.show` verb. |
| `instrukt sessions link --remove <a> <b>` | Remove the link through the runtime mapping; report a missing capability when unsupported. |
| `instrukt sessions resolve-start <procedure>@<project> --session <runtime:id> \| --none` | Resolve a standing agent's start whose result never arrived: bind the session the runtime did create, or declare that none was, allowing a fresh start. |
| `instrukt sessions conversation <id>` | Read the identified session's conversation through that mapping. |

Runtime verbs receive per-call input as follows:

- Per-call inputs (the opening message, sent text, another session's id) are
  appended to a `command` as separate arguments, never spliced into the
  command text; their content may be anything. An `agent` instruction is
  printed with those inputs after it, one per line.
- `start` also receives `INSTRUKT_WORKTREE`, `INSTRUKT_AGENT` and
  `INSTRUKT_THINKING` in its environment.
- `start` prints the session id, then the worktree path when one was
  requested; no path means the mapping lacks worktree support.
- The opening message names the procedure's closure read (``Run `instrukt docs
  get <procedure> --closure` …``), then the bound parameters, then, for a task
  run, `occurrence: <RFC 3339>` naming the scheduled occurrence, then the brief.

The mapping is global configuration, not executable instructions supplied by
a cloned project's configuration. Configuration interpolation expands each
verb's template or instruction at call time; `{runtime.session_id}` in a verb
is the session the call names. The active runtime inside an agent is the first
configured entry whose `session_id_env` is set. A containing runtime precedes
the agent runtime it hosts when both identify the caller. With no matching
entry, a caller-relative `runtime.*` lookup fails clearly.

An OS-started runner uses the runtime the developer chose: global
`runner.runtime`, naming an entry of `runtimes` or an entry instrukt ships,
and defaulting to the first entry in `runtimes`. No code detects or chooses a
runtime: with neither set there is no runtime, and every feature that needs
one is unavailable. Each runtime entry declares `name` and `session_id_env`,
and fills any of the verbs `start`, `send`, `alive`, `conversation`,
`notify`, `link`, `end`, `end_self`, `available`, `model`, `history.search`
and `history.show`, each with its kind
(`project/instrukt/spec/feature/configuration/layered-configuration`).
An entry with `agent: <name>` is an agent CLI; its git author comes from
`agents.<name>.git_author`.
One runner per computer uses its own runtime; targeting another computer's
sessions is outside this contract.

Every runtime is reached through its mapping entry's verbs alone. instrukt
has no code path for a particular runtime: an entry instrukt ships is driven
by the same facade, through the same verbs, as one a developer maps
themselves.

### Verb kinds

Each verb a runtime entry fills has one of two kinds:

| Kind | What `instrukt sessions <verb>` does |
| --- | --- |
| `command` | Runs the command template with the call's inputs and reports its result. |
| `agent` | Prints the instruction, with the call's inputs, for the calling agent to carry out with its own runtime's tools, and exits with the distinct "yours to do" status, which `instrukt sessions -h` names. instrukt runs nothing. |

The "yours to do" status is neither success nor failure: the call is complete
once the agent has carried out the instruction it printed. With `--json`, the
result names its kind and, for `agent`, the instruction. A call that expands
to several verbs, as `end --todo` does, prints every instruction and exits
with "yours to do" when any of them is `agent`.

`start`, `send`, `alive`, `conversation`, `notify`, `available` and `model`
are run by instrukt itself, including by an OS-started runner with no agent
to hand an instruction to, so they take kind `command` only. `end`,
`end_self`, `link`, `history.search` and `history.show` take either kind.

### Verb-gated features

A feature is available only when the runtime the developer chose fills every
verb it needs. Switching a feature on otherwise is refused, naming each
missing verb and the runtime entry it is missing from; the setting is left
unchanged. `instrukt config validate` reports an enabled feature whose
verbs the chosen runtime does not fill, with the same naming.

| Feature | Switched on by | Verbs it needs |
| --- | --- | --- |
| Standing agents | `modules.events: true` | `start`, `send`, `conversation`, `alive` |
| Todo dispatch | `modules.todos: true` | `start` with worktree support, `send`, `conversation`, `alive` |
| Recurring tasks | `modules.todos: true` | `start`, `conversation`, `alive` |
| Anomaly notification | `runner.notify` unset (the runtime's notification) | `notify` |
| Session status | always on; `instrukt sessions alive` | `alive` |
| Linked-session input | always on; `instrukt sessions link` | `link` |
| History lookup by agents | always on; `instrukt sessions history search`, `instrukt sessions history show` | `history.search`, `history.show` respectively |

Standing agents, todo dispatch and recurring tasks observe their sessions'
status for reuse and completion, so they need `alive` as well as the read.
An always-on command whose verb is missing refuses with the missing verb, as
`link` does. `runner.notify` set to a command makes anomaly notification
available without the runtime's `notify`. Ending a session is not a gated
feature (`Ending sessions`).

### Ending sessions

Ending is optional: it matters only for a runtime that holds resources per
session. A runtime entry that leaves `end` empty needs no ending:
`instrukt sessions end` reports that the runtime needs no ending, does
nothing and succeeds.

- `instrukt sessions end <id>` ends one session through `end`.
- `instrukt sessions end --todo <slug>` expands the todo's child list from
  instrukt's own session bindings (`sessions.bound`) and calls `end` once per
  bound session, leaving out the caller's own session. It checks no claim;
  the procedure that calls it confirms release first.
- `instrukt sessions end self` resolves the caller from the active runtime's
  `session_id_env` and ends it through `end_self`, or through `end` with the
  caller's id when `end_self` is empty. Outside any mapped runtime it fails,
  as every caller-relative lookup does. Ending itself is always the caller's
  last step.

Every ending goes through `instrukt sessions end`; doctrine names that
command and nothing the runtime offers.

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

instrukt calls the runtime only through the mapping's verbs. A verb the
chosen runtime does not fill leaves its features unavailable
(`Verb-gated features`); the capability is never rebuilt in instrukt.

### Agent-facing history verbs

`history.search` and `history.show` serve agents; the runner never calls
them. An agent calls them through `instrukt sessions history search <text>`
and `instrukt sessions history show <id>`, resolved for the runtime the agent
is running in. A snippet may also name the value through a
`<!-- @config: runtime.history.show -->` directive
(`project/instrukt/spec/feature/configuration/interpolation`), which
renders the verb's command or instruction.

- `history.search` finds earlier conversations by text and names their
  session ids; the agent supplies the text.
- `history.show` shows one conversation's messages; the agent supplies the
  session id, as a search named it or a record carries it.

Each is filled from that runtime's own search and show, as a `command` or as
an `agent` instruction. Doctrine never names a runtime's command or tool for
history; it names these calls. A runtime entry that leaves one empty makes its
lookup a refusal naming the missing verb. `instrukt sessions conversation <id>`
stays the facade's read of a recorded session through the entry's
`conversation` verb.

### Shipped T3 Code entry

instrukt ships one runtime entry, `t3`, for T3 Code. A developer chooses it
with `runner.runtime: t3`; nothing selects it for them. Chosen, it is part of
the effective `runtimes` as shipped, after any declared entries, unless the
global layer declares an entry named `t3`, which then replaces it
(`project/instrukt/spec/feature/configuration/layered-configuration`).

The entry's `command` verbs are filled from T3's host command `t3 thread`,
whose subcommands are `start`, `send`, `show` and `list`. They reach the T3
server running on the same computer without a sign-in. Its `agent` verbs are
instructions for an agent running in a T3 thread, carried out with T3's
thread tools.

| Key | Filled with |
| --- | --- |
| `session_id_env` | `T3CODE_THREAD_ID`, which T3 sets in every thread's shells |
| `start` | `t3 thread start .`, run in the project checkout, with the opening message appended. A worktree request adds `--worktree` with the project's default branch as its base and `--branch` with the requested name, and prints the thread's worktree path, read from `t3 thread show --json`, on the line after the id. |
| `send` | `t3 thread send {runtime.session_id}`, with the text appended. A busy thread receives it as its next turn; nothing is interrupted. |
| `alive` | `t3 thread show --json --messages 0 {runtime.session_id}`, translated into the five states (below). |
| `conversation` | `t3 thread show --messages 100 {runtime.session_id}` |
| `history.show` | `command`: `t3 thread show --messages 100`, with the session id appended |
| `history.search` | `agent`: search T3's threads for the supplied text with the T3 thread search tool (`t3_thread_search`) and read the thread ids it names |
| `end` | `agent`: settle thread `{runtime.session_id}` with the T3 thread organize tool (`t3_thread_organize`, action `settle`) |
| `end_self` | `agent`: settle this thread with `t3_thread_organize`, action `settle`, naming no thread; T3 applies it when this turn ends, so it is the last thing done |
| `notify`, `link`, `available`, `model` | empty: T3 offers nothing that raises a notification, links threads, checks a provider, or reports a model and version |

Every other verb in the table is `command`. On T3 ending is implicit: T3
stops an idle thread's provider process itself, and only a live thread holds
one. The shipped `end` and `end_self` settle the thread, so the developer's
thread list shows the work as done; neither frees anything T3 would keep.

So on T3 standing agents, todo dispatch, recurring tasks, session status,
ending and history lookup are available. These are unavailable:

- anomaly notification through the runtime: unavailable unless `runner.notify`
  is set to a command;
- linked-session input.

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

The runtime entry's `alive` verb returns one of these five states. A
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
false, then call `instrukt sessions end --todo <slug>`. End the lane with
`instrukt sessions end self` only after its obligations finish, as its last
step; preserve an unfinished human conversation.

`instrukt sessions alive <id>` prints one of the five statuses; `run` and
`send` print the session id they used; every facade command accepts
`--json`. A status that cannot be observed is reported as such, never as
`gone`. A procedure launch with `run` loads the named procedure, binds `--args` according to its declared
parameters, and carries `--brief <file|->` as additional briefing content
(from the file or standard input). These flags also apply when `--worktree`
is supplied. Worktree support is a capability of the runtime's `start` verb,
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
  Then instrukt searches that runtime for it, or tells the agent how to search with its own runtime's tools
  And shows a conversation the search finds
  But when the runtime it is running in leaves history search empty
  Then it is told which verb is missing, and no search is made
```

#### UC-SS5: A developer switches on a feature their runtime cannot carry

```gherkin
  Given a developer's chosen runtime leaves empty a verb a feature needs
  When they switch that feature on
  Then the change is refused, naming each missing verb and the runtime it is missing from
  And their configuration is unchanged
  But when their runtime fills every verb that feature needs
  Then the feature is switched on
```

#### UC-SS6: A developer who has chosen no runtime is never given one

```gherkin
  Given a developer has T3 Code installed but has chosen no runtime
  When they switch on work dispatch or standing agents
  Then the change is refused, saying that no runtime is chosen
  And no runtime is picked for them
```

#### UC-SS7: An agent told that an ending is its to do ends the sessions itself

```gherkin
  Given a developer's runtime fills ending as an instruction for the agent
  And an agent has released a todo whose child sessions are bound to it
  When the agent ends the todo's sessions and then its own through instrukt
  Then instrukt tells it, for each session, what to do with its own runtime's tools, and that doing it is the agent's
  And once the agent has done so, the child sessions and its own are ended in the runtime
  But when the developer's runtime needs no ending
  Then instrukt says so and nothing more is owed
```

## Known caveats

- T3 selects a provider only together with a model slug, and instrukt's agent
  names are not model slugs, so the shipped `start` selects none: dispatch's
  agent order does not choose the model of a thread started through the
  shipped entry.

## See Also

- project/instrukt/spec/feature/configuration/layered-configuration — choosing the runtime, the shipped entry and the runtime entry's verb shape
- project/instrukt/spec/feature/configuration/interpolation — the active-runtime view agents read verbs through
- project/instrukt/spec/feature/runner/anomaly-notification — `runner.notify` when the runtime offers no notification
