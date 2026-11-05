---
description: Read-time expansion of cross-key references, environment variables and the selected adapter's caller identity; documentation reads configured values through @config.
delivered_by: [instrukt-first-build]
---
# Interpolation — Spec

## What it is

`instrukt config get` expands configured values when read, in the caller's
environment. A snippet's `@config` directive uses the same in-process lookup
against the effective configuration of the reader's project. Expanded
caller-bound values are never written back or stored in published snippets.

## Canonical fields

### References

| Form | Expands to |
| --- | --- |
| `{other.key}` | The value at that dotted configuration path, itself expanded. |
| `${VAR}` | The caller's environment variable `VAR`; an unset variable is an error. |
| `{runtime.session_id}` | The caller id recognised by the explicitly selected runtime adapter. |

Expansion occurs on every read. A self-reference or cycle fails and names the
cycle. `runtime.session_id` is the sole derived runtime value; runtime verb
lookups, templates and instructions are not configuration paths. A snippet
needing a recorded conversation names `instrukt sessions conversation <id>`
rather than interpolating a command.

### Caller identity

The runtime is chosen by global `runner.runtime`, never inferred from an
environment variable. That adapter recognises its caller context; T3 reads
`T3CODE_THREAD_ID`. This only identifies a caller inside the chosen runtime; it
neither selects another runtime nor configures session operations.

Without a selected adapter or its caller context, `config get
runtime.session_id` and references to it fail non-zero with a clear reason.
Ordinary configuration reads still work. `@config` resolves values, including
this identity where available, under the same rules.

### Use cases

#### UC-VI2: An agent reads a configured value in its current environment

```gherkin
Given a developer's configured value refers to another key and an environment variable
When an agent reads that value through config get or a snippet's config directive
Then it receives the expanded value for its current environment
And the stored configuration is unchanged
```

#### UC-VI3: An agent identifies its session within the chosen runtime

```gherkin
Given a developer has explicitly selected the T3 runtime adapter
And an agent's environment identifies its current T3 thread
When the agent reads runtime.session_id
Then it receives that native thread id
But outside that runtime's caller context
Then the read fails without guessing a runtime or session
```

#### UC-VI1: An agent obtains the command that reads its own conversation (to retire)

UC-VI1 is to retire: the superseded scenario remains only while its existing
test bindings are removed by the builder.

```gherkin
Given a developer's machine maps the runtimes it uses, with a command to read a session's conversation
When an agent running in one of those runtimes asks for the command to read its conversation
Then it receives the command for its own session in the runtime it is running in
But when it runs in no mapped runtime
Then it is told that no configured runtime matches, and no command is given
```

## Known caveats

- UC-VI1 is to retire: obtaining a configured conversation command is replaced
  by the adapter-backed sessions conversation command.
- `runtime.<verb>` and `runtimes` are unknown configuration paths. The
  pre-release template mapping is neither read nor migrated.

## See Also

- project/instrukt/spec/domain/configuration
- project/instrukt/spec/feature/configuration/layered-configuration
- project/instrukt/spec/feature/docs/progressive-retrieval
- project/instrukt/spec/feature/runner/session-continuity
