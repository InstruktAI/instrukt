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
| `{runtime.<operation>}` | The string at `runtimes.<selected runner.runtime>.<operation>`. |

Expansion occurs on every read. A self-reference or cycle fails and names the
cycle. The selected-runtime instruction alias and caller identity are distinct
read-time values. Instruction lookup needs explicit runtime selection but no
caller id. The stored instruction is returned as data. Code performs only named lookup
and ordinary interpolation; it embeds no agent-native instruction, fallback
command, default wording or tool-selection branch. Unattended adapter operations
remain compiled code under their separate contract.
Workflow assignment fields (brief, role, attempt, title/key, base and branch)
are passed separately to the agent, not substituted as shell or configuration
expressions into this prose.

A procedure renders an operation with a standalone directive, for example:

```markdown
<!-- @config: runtime.dispatch -->
```

Every agent-action consumer uses this same `@config: runtime.<name>` form.
The complete names, required inputs and shipped T3 entry values are owned by
`project/instrukt/spec/feature/configuration/layered-configuration`. A value may
render a native instruction or an implemented instrukt route for a missing native
capability; the consumer carries neither choice in its prose or code.
An absent runtime, entry or operation fails lookup visibly; no facade or runtime
is silently chosen. These context-dependent aliases remain invalid in compiled
agent files; procedures retrieve them on demand. A native child's ending is
runtime delivery, with no instruction or instrukt-produced input.

### Caller identity

The runtime is chosen by global `runner.runtime`, never inferred from an
environment variable. That adapter recognises its caller context; T3 reads
`T3CODE_THREAD_ID`. This only identifies a caller inside the chosen runtime; it
neither selects another runtime nor configures session operations.

Without a selected adapter or its caller context, `config get
runtime.session_id` and references to it fail non-zero with a clear reason.
Ordinary configuration reads still work. `@config` resolves values, including
this identity where available, under the same rules.

## Use cases

### UC-VI2: An agent reads a configured value in its current environment

```gherkin
Given a developer's configured value refers to another key and an environment variable
When an agent reads that value through config get or a snippet's config directive
Then it receives the expanded value for its current environment
And the stored configuration is unchanged
```

### UC-VI3: An agent identifies its session within the chosen runtime

```gherkin
Given a developer has explicitly selected the T3 runtime adapter
And an agent's environment identifies its current T3 thread
When the agent reads runtime.session_id
Then it receives that native thread id
But outside that runtime's caller context
Then the read fails without guessing a runtime or session
```

### UC-VI1: Reading a configured conversation command (retired)

Retired: agent conversation reads use selected-runtime action values; this id is not reused.

### UC-VI4: Render a runtime action without a caller session

```gherkin
Given a developer explicitly selected a runtime with a configured delegation instruction
And no caller session identity is available
When a procedure renders runtime.dispatch through its config directive
Then the instruction comes verbatim from that runtime's configuration entry
And the same directive can render another runtime's configured instrukt fallback route
And no session is started by the read
But an absent runtime or instruction is reported as unresolved rather than guessed
```

## Known caveats

- UC-VI1 is retired: obtaining an executable conversation command is outside
  the contract. Configured action values do not revive this retired goal.
- Only `runtime.session_id` and `runtime.<operation>` are derived
  runtime paths. Legacy executable verb mappings are neither read nor migrated.
- Current-thread listing and metadata search do not release archive/history search; A3's history templates remain outside this contract.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/docs/progressive-retrieval](../docs/progressive-retrieval.md)
- [project/instrukt/spec/feature/runner/session-continuity](../runner/session-continuity.md)
