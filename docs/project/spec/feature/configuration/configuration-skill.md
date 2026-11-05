---
description: One runtime-neutral agent skill configures participating computers through local instrukt commands, with runtime-specific coordination and verification.
delivered_by: [instrukt-first-build]
---

# Configuration Skill — Spec

## What it is

The `instrukt-configure` Agent Skill guides a developer through configuring
one computer or a cluster. Its runtime-neutral entry settles the desired
outcome, reads existing state, assigns each computer its local configuration
work and verifies the result. Runtime-specific instructions coordinate local
agents; instrukt configuration and installation commands remain local.

## Canonical fields

### Skill package and entry

The package consists of `skills/instrukt-configure/SKILL.md` and
`skills/instrukt-configure/runtime/t3.md`. The entry's Agent Skills frontmatter
uses `name`, `description` and optional `license`, `compatibility`, `metadata`;
the description is at most 1024 characters. Runtime instructions are supporting
Markdown within the same package.

The developer supplies a checkout or a copy of this complete skill directory
to the configuring agent on each participating computer. This is a source
artifact entry, independent of a plugin, package distributor or release
installer. An absent instrukt binary or required command is a reported
prerequisite, not an installation mechanism supplied by this skill.

### Configuration facts and agreement

Each computer owns its global layer, `~/.instrukt/instrukt.yaml`. Projects own
their project layers. There is no shared instrukt layer, fleet state service
or new project-owner-machine routing key. Configuration management outside
instrukt distributes agreed facts, including any operator-managed dotfiles.

| Fact | Agreement and scope |
| --- | --- |
| `ingress.computer` | All participants use the same existing stable instrukt machine ID for the one ingress. A runtime environment ID or hostname does not substitute for it. |
| `redis.cluster.url` | All event participants reach the same logical Redis database and stream namespace. URL spellings may differ when they resolve to that same store. |
| `redis.cluster.username`, `redis.cluster.password` | Machine-local credential references grant the required access to that common store; credentials and reference spellings need not match. |
| Project identity and placement | The coordination brief records which computer holds each participating project and its local checkout. Project `project` identity and source declarations identify the same project wherever needed. This is a configuration-management fact, not a new YAML key or a replacement for todo claims. |
| Ingress project homes | Every project declaring inbound sources has its adopted checkout on ingress; its signing references resolve in that project's `.env` there. External procedure activation remains on ingress under the intake contract. |
| `ingress.port` | The receiver and its local edge agree on this port; other computers need not share it. |

These are the cross-computer and edge-local agreements in
`project/instrukt/spec/feature/configuration/layered-configuration` and
`project/instrukt/spec/feature/runner/module-installation`. Local Redis sockets,
local credentials, runtime selection, tick policy and project-specific module
overrides remain local. Secret material stays at its declaring computer;
coordination carries references and verification findings.

### Runtime-neutral configuration work

The skill reads configuration before asking questions and asks the developer
only for unresolved intent or facts instrukt cannot answer: participating
computers, desired module enablement, ingress choice, operator-supplied cluster
endpoint, project placement and an unset runtime choice. Existing choices are
kept unless the requested outcome changes them. An unset runtime means no
runtime; the agent's host does not select one.

The selected `runner.runtime` routes coordination to the packaged
`runtime/<runtime>.md` when present. Each target still retains its own runtime
choice. Without an available route, the developer runs the same skill locally
on each computer with the agreed facts. Local core configuration remains usable
without a runtime; runtime-dependent enablement follows capability validation.

On each computer the agent uses `instrukt -h` and command `-h` to discover the
installed surface, `instrukt config get [--global] [--json]` and
`instrukt config get --revision [--global]` to read settings and revisions,
then `instrukt config patch --revision <rev> [--global] --from-file <file>`
and `instrukt config validate [--global]` to record and check only the intended
differences. Missing layers use `instrukt config init [--global]`. A stale
revision requires a fresh read and recomputed difference.

Only developer-enabled modules authorize their infrastructure. After recording
desired settings, the local agent uses `instrukt install [--module todos|events]`
to converge demanded infrastructure, then `instrukt install --check` with the
same scope and relevant `instrukt ticks status` / `instrukt events status`.
Disabled-module withdrawal uses the installation contract's full demand read.
Reruns preserve matching settings and working resources, and repair differences
through the same commands. Installation findings remain distinct from valid
configuration; they retain their named repair doors.

### T3 coordination

The T3 route uses T3's linked-environment capabilities and dispatches one local
configuration task per participating environment, including local work. Each
task receives the agreed facts, that machine's intended settings and project
paths, the skill package and a report destination. It performs only its
computer's configuration and verification, returning references, completed
changes and unmet prerequisites. The coordinator compares those reports for
agreement and leaves failed or inaccessible computers explicitly incomplete.

T3 publishes remote access through its environment MCP interface in **Outside
agents**. Its host `t3 -h` describes environment linking; the published thread
help describes host-local thread operations. Runtime instructions discover the
capabilities and permission limits supplied by that T3 connection. A host-local
thread start does not select a remote computer; remote tasks use the target
linked environment. Harness-specific tool names are outside this skill's text.

### Use cases

#### UC-CS1: Configure a two-computer cluster

```gherkin
Given a developer wants two linked computers to use instrukt events
And has selected one ingress and supplied the cluster connection and project placement
When the developer runs the configuration skill with T3 coordination
Then one local configuration task per computer records and verifies its intended settings
And both computers agree on ingress identity, the logical cluster store and project placement
And only ingress installs the demanded receiver with its declaring project homes
And the developer receives each computer's configuration and infrastructure verification results
```

#### UC-CS2: Repair one computer that drifted

```gherkin
Given a developer's cluster has one computer whose configuration or owned infrastructure differs from the agreed setup
When the developer reruns the configuration skill to repair that computer
Then the agent reads its current configuration and repairs only the intended differences
And matching settings, working resources and retained consumer progress are preserved
And verification reports any remaining unmet prerequisite separately from completed changes
```

#### UC-CS3: Configure a computer without a runtime route

```gherkin
Given a developer has a participating computer without an available runtime coordination route
When the developer asks the skill to configure the cluster
Then the skill supplies the agreed facts and local instructions for that computer
And the developer can run the same skill there to configure and verify it locally
And cluster completion remains pending until that computer's verification is available
```

#### UC-CS4: Keep module infrastructure tied to the developer's enablement

```gherkin
Given a developer wants only core configuration on a computer
When the configuration skill records and verifies that setup
Then it keeps module enablement at the developer's chosen settings
And it installs no module infrastructure for that core-only project
But when the developer enables a module
Then the local agent records that choice and installs and checks only the infrastructure demanded by enabled projects
```

#### UC-CS5: See incomplete cluster setup when a computer is unavailable

```gherkin
Given a developer requests configuration across participating linked environments
And one environment is unreachable or lacks permission to run a local task
When the configuration skill collects the results
Then it reports completed configuration and verification for reachable computers
And it names the incomplete computer and the prerequisite for continuing there
And it offers the local skill path for that computer without claiming cluster completion
```

## Known caveats

- Distribution and binary installation are outside this package's contract.
  The skill directory must already be readable where an agent runs it.
- Agreements are checked from participating agents' observations, not an
  atomic cluster transaction. Concurrent changes require rereading revisions;
  partial completion is reported per computer.
- There is no dedicated project-owner-machine configuration key. Project
  placement follows existing adopted homes and intake/claim contracts.
- Local command availability can lag this pending specification. Missing
  commands leave the relevant work incomplete.
- Linking, authentication, edge TLS/provider registration and provision of
  the cluster Redis endpoint remain operator prerequisites.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](layered-configuration.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](configuration-editing.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](../runner/single-ingress-webhook-termination.md)
- [general/policy/agent-facing-text](../../../../general/policy/agent-facing-text.md)
