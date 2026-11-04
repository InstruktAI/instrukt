---
description: Two-phase interpolation — `instrukt config get` expands cross-key references, environment variables and the active runtime's session id at call time; docs read values through read-time commands.
delivered_by: [instrukt-first-build]
---

# Interpolation — Spec

## What it is

A configured value may depend on other values, on the caller's environment and
on the runtime session the caller is in. Such values are resolved in two
phases. First, `instrukt config get` expands them at the moment it is called,
in the caller's environment. Second, documentation never carries a template
syntax of its own: a snippet asks for a value with the `@config` directive,
resolved through `instrukt config get`'s resolution when the snippet is read
(`local/projects/instrukt/spec/feature/docs/progressive-retrieval`). Nothing published
carries a session-bound value.

## Canonical fields

### References

| Form | Expands to |
| --- | --- |
| `{other.key}` | the value at that dotted path of the same configuration `get` reads from, itself expanded |
| `${VAR}` | the caller's environment variable `VAR` |
| `{runtime.<field>}` | that field of the active runtime's mapping entry |
| `{runtime.session_id}` | the value of the environment variable the active runtime's `session_id_env` names |

`${VAR}` has no default form. Expansion happens on every `get`; nothing is
cached and nothing expanded is written back.

The runtime mapping itself is the exception. Its entries hold templates the
runner fills per session, not values: reading the mapping (`config get
runtimes`, or any path under it) prints the entries as stored, with every
template unexpanded, `{…}` and `${VAR}` alike. `{runtime.*}` references
expand only through the active-runtime view, `config get runtime.<field>`.
Every other value expands on every `get`.

### Active runtime

The runtime mapping (`runtimes`, global layer) is an ordered list. The active
runtime is the first entry whose `session_id_env` is set in the caller's
environment. A runtime that hosts agent CLIs is listed before those CLIs,
because a session inside it also carries the hosted CLI's own variable.

When no entry's variable is set, any `runtime.*` path or reference (the
active-runtime view, not the stored mapping) fails non-zero with an error stating that no configured runtime's session variable
is set; no runtime is guessed.

Example (values are illustrative):

```yaml
runtimes:
  - name: thread-host
    session_id_env: THREAD_HOST_ID
    conversation: "thread-host show {runtime.session_id} --json"
  - name: agent-cli
    session_id_env: AGENT_CLI_SESSION_ID
    conversation: "agent-cli history {runtime.session_id}"
```

Inside a `thread-host` session, `instrukt config get runtime.conversation`
prints `thread-host show <that session's id> --json`.

### Use cases

#### UC-VI1: An agent obtains the command that reads its own conversation

```gherkin
Given a developer's machine maps the runtimes it uses, with a command to read a session's conversation
When an agent running in one of those runtimes asks for the command to read its conversation
Then it receives the command for its own session in the runtime it is running in
But when it runs in no mapped runtime
Then it is told that no configured runtime matches, and no command is given
```

## Known caveats

- An unset `${VAR}` is an error naming the variable; a self-reference or reference cycle is an error naming the cycle.

## See Also

- local/projects/instrukt/spec/domain/configuration
- local/projects/instrukt/spec/feature/docs/progressive-retrieval — the `@config` directive that reads configured values into snippets
