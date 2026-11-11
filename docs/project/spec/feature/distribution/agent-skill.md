---
description: "The instrukt skill — instrukt's one skill, kept in its own repository and installed into every agent CLI on a computer with one skills command pinned to a release; every visit reads `instrukt status` and converges on what the human asks: adopt a repository, switch on a module, or configure one computer or several, changing only what differs."
delivered_by: [release-installer]
---
# Agent Skill — Spec

## What it is

A developer tells their agent "install instrukt" and gets instrukt: the
binary and one place to start, in every agent CLI they use. That place is one
skill, the cockpit, through which an agent sets instrukt up and changes it.
It is an Agent Skill (`agentskills.io/specification`) kept in instrukt's own
repository and installed with the `skills` command (`skills.sh`), which puts
it where each agent CLI on the computer reads skills. instrukt adds no
plugin, no hook, no background process and no setting of its own: every
choice the human makes lands in instrukt's configuration, where the CLI
reads it.

## Canonical fields

### Where the skill lives

| Path in `InstruktAI/instrukt` | Content |
| --- | --- |
| `skills/instrukt/SKILL.md` | the cockpit skill |
| `skills/instrukt/references/configure.md` | the skill's configuration reference, read when the request turns to configuration |

- instrukt has exactly one skill. Configuration is part of it, as a
  reference file inside the skill's directory that the cockpit opens on
  demand (the Agent Skills specification's `references/` layout); there is no
  second skill.

- Its frontmatter carries only fields the Agent Skills specification
  defines: `name: instrukt`, `description`, `license: MIT`,
  `compatibility` (needs the `instrukt` binary, which the skill installs, and
  a POSIX shell), and `metadata.version`, the instrukt release it belongs
  to, without the `v`.
- `metadata.version` changes only in a release commit, so a release tag pins
  the skill and the binary of one release together
  (`project/instrukt/spec/feature/distribution/release-installer`).
- The repository holds no plugin manifest, marketplace file or hook for any
  agent CLI.

### Install with one sentence

"Install instrukt" asks the agent to follow instrukt's `README.md`, which
opens with two commands for a human and for an agent alike:

```sh
curl -fsSL https://github.com/InstruktAI/instrukt/releases/latest/download/install.sh | sh
npx skills add InstruktAI/instrukt@v<version> --skill instrukt -g -y
```

The release installer installs the binary and ends by printing the second
command with its own release in place of `<version>`. That command installs
the skill at user scope into every agent CLI the `skills` command finds on
the computer, so it is available in every project. An agent CLI loads it in
its next session. A developer who installs the skill first still gets the
binary: the cockpit installs it on its first visit (below).

### The cockpit skill

One hand-authored skill, and it is idempotent: every visit reads the current
state and converges on the result the human asked for, changing only what
differs and keeping everything that already matches, so a visit to a computer
that is already right changes nothing. Its `description`, within the specification's
1,024 characters, tells any agent to open it whenever the human wants
instrukt installed, set up, extended or changed, or asks what instrukt is
doing on this computer, including when instrukt is not installed or not set
up yet, and that `instrukt -h` lists the commands. Every visit runs the same
way, whatever was done before:

1. **Read the state.** Run `instrukt status --json`
   (`project/instrukt/spec/feature/publication/instrukt-status`).
   When there is no binary, install it with the release installer pinned to
   the skill's `metadata.version`, then read the state. When the binary is
   older than the skill's release, say so and offer the same pinned install.
2. **Act on what the human asked**, against that state:
   - **Adopt this repository.** When status reports it adopted, say so and
     what it publishes. Otherwise run `instrukt init` and show what it wrote and
     the claims it reports. When init kept a hand-written `AGENTS.md`
     (`.instrukt/adopted/AGENTS.md`), go through it with the human before the
     first commit: its purpose into the master's `## Purpose`, each
     instruction into a project snippet of its type with its index entry
     (`instrukt docs schema docs/project` names the types), then remove the
     kept copy. Leave the first commit to main, which starts publication, to
     the human's word.
   - **Adopt another repository.** The same, in the repository the human
     names.
   - **Switch on a module.** For todos, when status reports that this
     repository's code is not on GitHub, say first that todos need the
     project's code on GitHub, and go no further. When status reports a
     feature the module needs unavailable on the configured runtime, name the
     missing adapter capabilities or absent runtime choice and keep the module
     off until a configured adapter supports it. Otherwise ask
     what the module needs and only the human can say, write it with
     `instrukt config patch`, check it with `instrukt config validate`, and
     register the wake-up with `instrukt install` when the module runs
     through the runner.
   - **Reconfigure.** Change layered settings with `instrukt config patch`
     in their owning layer and validate. For model selection, roles, GitHub
     Apps, secrets and configuration across computers, the cockpit reads its
     configuration reference and follows it (Configuration, below).
   - Anything else about instrukt: `instrukt -h` and `instrukt docs`.
3. **Show the result** by reading `instrukt status` again.

The setup is an interview the agent guides. It asks the human only what
instrukt cannot answer: which runtime to call (no code detects or chooses
one), which docs repository to adopt when the computer publishes none, the
agent preferences and git authors, which modules, and the values a module
needs. Everything instrukt can answer comes from status and `-h`, never from
the human. Layered settings and secrets use instrukt's CLI; model profiles and
role orders use the `agents.yaml` contract
(`project/instrukt/spec/feature/configuration/agent-selection`). The skill never rewrites
itself and keeps no state: the human's choices live in instrukt's
configuration, so the same skill is opened again for any later change.

Once the computer is set up, the global agent files sync renders
(`project/instrukt/spec/feature/publication/agent-entrypoints`) tell every
session about instrukt's documentation; the skill stays the way in for
changing instrukt.

### Configuration

The configuration reference configures one computer or several. It settles
the desired outcome, reads existing state, assigns each computer its local
configuration work and verifies the result. Runtime-specific instructions
coordinate local agents; instrukt configuration and installation commands
remain local. An absent instrukt binary or required command is a reported
prerequisite for the affected work.

#### Configuration facts and agreement

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
| Ingress project homes | Every project declaring inbound sources has its adopted checkout on ingress; its signing references resolve in the ingress computer's owner-only `~/.instrukt/secrets.env`. External procedure activation remains on ingress under the intake contract. |
| `ingress.port` | The receiver and its local edge agree on this port; other computers need not share it. |

These are the cross-computer and edge-local agreements in
`project/instrukt/spec/feature/configuration/layered-configuration` and
`project/instrukt/spec/feature/runner/module-installation`. Local Redis sockets,
local credentials, runtime selection, tick policy and project-specific module
overrides remain local. Secret material stays at its declaring computer;
coordination carries references and verification findings.

#### Runtime-neutral configuration work

The skill reads configuration before asking questions and asks the developer
only for unresolved intent or facts instrukt cannot answer: participating
computers, desired module enablement (blueprint, todos, events and default-on
history), ingress choice, operator-supplied cluster endpoint, project placement
and an unset runtime choice. Existing choices are kept unless the requested
outcome changes them. An unset runtime means no runtime; the agent's host does
not select one.

The selected `runner.runtime` chooses the configured action values rendered
by `general/procedure/ai-to-ai-operations`, retrieved on demand by the skill.
Each target still retains its own runtime choice. Without an available route,
the developer runs the same skill locally on each computer with the agreed
facts. Local core configuration remains usable without a runtime;
runtime-dependent enablement follows capability validation.

On each computer the agent uses `instrukt -h` and command `-h` to discover the
installed surface, `instrukt config get [--global] [--json]` and
`instrukt config get --revision [--global]` to read settings and revisions,
then `instrukt config patch --revision <rev> [--global] --from-file <file>`
and `instrukt config validate [--global]` to record and check only the intended
differences. Missing layers use `instrukt config init [--global]`. A stale
revision requires a fresh read and recomputed difference.

Offer history like the other modules, explaining its default-on setting and
independent cluster Redis demand for merged search. A core-only setup explicitly
disables history. Local history search needs no Redis.

Only enabled modules authorize their infrastructure. After recording
desired settings, the local agent uses `instrukt install` with the module scope
supported by `project/instrukt/spec/feature/runner/module-installation`,
including blueprint to converge demanded infrastructure, then
`instrukt install --check` with the same scope and relevant
`instrukt ticks status` / `instrukt events status`. Disabled-module withdrawal
uses the installation contract's full demand read. Reruns preserve matching
settings and working resources, and repair differences through the same
commands. Installation findings remain distinct from valid configuration; they
retain their named repair doors.

#### Agent profiles and readiness

Read `~/.instrukt/agents.yaml` and show the current role/profile order before
changing model selection. Detect ACP instances already configured in T3, ask
where the developer wants them in that order, and write their profiles and order
changes. For a supported harness not ready, guide login or key setup and wait
for readiness. Keep model-provider keys with T3. instrukt creates no T3 provider
instance and implements no ACP. Selection and upgrade semantics belong to
`project/instrukt/spec/feature/configuration/agent-selection`.

Write instrukt-owned signing and Redis secrets only through `instrukt secrets
set` or `instrukt secrets generate`, without echoing values. Every computer's
interactive and scheduled paths read its one protected machine-local file
(`project/instrukt/spec/feature/configuration/owned-files`).

#### Roles and GitHub Apps

Read the effective role map, classes and scoped bundles together with model
selection. Preserve the shipped event-handler/default fallback and validate
changed action and bundle references under
`project/instrukt/spec/feature/configuration/roles-and-capabilities`.

GitHub setup follows `project/instrukt/spec/feature/work/github-apps`: reuse the
existing three TeleClaude Apps where installed, otherwise guide their creation
and installation, verify the actual App/bot identities and repository/board
permissions, and record machine-global public profiles plus per-role App
references in the existing `roles` map. Write App keys only through
`instrukt secrets set`. Verify installation-token issuance and distinct author/
reviewer actors without exposing keys or tokens. A missing permission or failed
identity/issuance check leaves setup visibly incomplete with its repair door.

#### Runtime coordination

The shared operation procedure renders `runtime.discover_environments`,
`runtime.connect_environment` and `runtime.dispatch_remote` through `@config`.
Their entries carry native or capability-gap route instructions; the skill and
workflow contain no runtime-specific call or packaged runtime route. The names,
required inputs and T3 values are owned by layered-configuration.

The coordinating agent assigns one local brief per participating computer,
including local work. Each agent receives agreed facts, intended settings,
project paths, the skill and a report destination. Each performs its own
computer's configuration and verification and returns completed changes and
unmet prerequisites. The coordinator compares reports for agreement and retains
partial success; inaccessible computers stay incomplete. Runtime environment
addresses and instrukt machine IDs remain separate observed facts.

### Updates

`npx skills add InstruktAI/instrukt@v<version> --skill instrukt -g -y` with a
newer release replaces the installed skill; its cockpit then offers the
matching binary. The skill never upgrades the binary on its own.

## Use cases

### UC-CP1: A developer installs instrukt into their agents with one sentence

```gherkin
Given a developer uses one or more agent CLIs on a supported computer without instrukt
When they tell their agent "install instrukt"
Then the instrukt binary of the latest release is installed
And the instrukt skill of the same release is installed for every agent CLI on the computer
And in their next session, in any of those agent CLIs, opening the skill reports instrukt's state
```

UC-CP2 is retired: no startup hook announces instrukt.

### UC-CP3: A developer adopts a repository through an interview the agent guides

```gherkin
Given a developer has instrukt installed and a repository that is not adopted
When they ask their agent to adopt the repository
Then the agent reads instrukt's state and asks them only what instrukt cannot answer
And instrukt adopts the repository, and the agent shows what was written and what the repository will claim on the computer
And nothing is committed until the developer says so
When they ask the same for a repository that is already adopted
Then the agent says it is adopted and what it publishes, and changes nothing
```

### UC-CP4: A developer switches on a module the runtime can carry

```gherkin
Given a developer's computer is configured with a runtime
When they ask their agent to switch on standing agents
And the selected runtime adapter supports every capability standing agents need
Then the agent asks only what the module needs from them and instrukt switches it on
And the state the agent reports afterwards shows the module on
But when the adapter lacks a required capability
Then the agent names the missing capability, and the module stays off
```

### UC-CP5: A developer changes their setup later through the same skill

```gherkin
Given a developer set instrukt up through the instrukt skill some time ago
When they ask their agent to change which runtime instrukt calls
Then the agent reads the current state, asks for the new choice and changes instrukt's configuration
And the skill itself is unchanged, and opening it again shows the new state
```

### UC-CP6: A developer who installed the skill first still gets the binary

```gherkin
Given a developer installed the instrukt skill but not the instrukt binary
When they ask their agent anything about instrukt
Then the agent installs the binary of the skill's release and reports instrukt's state
```

#### UC-CS1: Configure a two-computer cluster

```gherkin
Given a developer wants two linked computers to use instrukt events
And has selected one ingress and supplied the cluster connection and project placement
When the developer asks the instrukt skill to configure the cluster using T3's configured action values
Then one local configuration task per computer records and verifies its intended settings
And both computers agree on ingress identity, the logical cluster store and project placement
And only ingress installs the demanded receiver with its declaring project homes
And the developer receives each computer's configuration and infrastructure verification results
```

#### UC-CS2: Repair one computer that drifted

```gherkin
Given a developer's cluster has one computer whose configuration or owned infrastructure differs from the agreed setup
When the developer opens the instrukt skill again to repair that computer
Then the agent reads its current configuration and repairs only the intended differences
And matching settings, working resources and retained consumer progress are preserved
And verification reports any remaining unmet prerequisite separately from completed changes
```

#### UC-CS3: Configure a computer without a runtime route

```gherkin
Given a developer has a participating computer without an available runtime coordination route
When the developer asks the instrukt skill to configure the cluster
Then the skill supplies the agreed facts and local instructions for that computer
And the developer can run the same skill there to configure and verify it locally
And cluster completion remains pending until that computer's verification is available
```

#### UC-CS4: Keep module infrastructure tied to the developer's enablement

```gherkin
Given a developer wants only core configuration on a computer
When the instrukt skill records and verifies that setup
Then it keeps module enablement at the developer's chosen settings
And it installs no module infrastructure for that core-only project
But when the developer enables a module
Then the local agent records that choice and installs and checks only the infrastructure demanded by enabled projects
```

#### UC-CS5: See incomplete cluster setup when a computer is unavailable

```gherkin
Given a developer requests configuration across participating linked environments
And one environment is unreachable or lacks permission to run a local task
When the instrukt skill collects the results
Then it reports completed configuration and verification for reachable computers
And it names the incomplete computer and the prerequisite for continuing there
And it offers the skill for that computer without claiming cluster completion
```

## Known caveats

- The `skills` command needs Node.js (`npx`). It is a third-party tool
  (Vercel Labs) and sends anonymous usage data unless `DISABLE_TELEMETRY=1`
  is set; instrukt neither depends on nor configures that.
- The skill is listed in no agent provider's own plugin store or directory.
- An unpinned `npx skills add InstruktAI/instrukt` installs the skill from
  the default branch, which may be ahead of the latest release; the pinned
  command is the one instrukt prints and documents.
- Agent CLI cloud sessions do not load skills installed on the developer's
  computer.
- Configuration agreements are checked from participating agents'
  observations, not an atomic cluster transaction. Concurrent changes require
  rereading revisions; partial completion is reported per computer.
- There is no dedicated project-owner-machine configuration key. Project
  placement follows existing adopted homes and intake/claim contracts.
- Linking, authentication, edge TLS/provider registration and provision of
  the cluster Redis endpoint remain operator prerequisites.

## See Also

- [project/instrukt/spec/domain/distribution](../../domain/distribution.md)
- [project/instrukt/spec/feature/distribution/release-installer](release-installer.md)
- [project/instrukt/spec/feature/publication/instrukt-status](../publication/instrukt-status.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
- [project/instrukt/spec/feature/repository/project-initialization](../repository/project-initialization.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](../configuration/configuration-editing.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/agent-selection](../configuration/agent-selection.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/runner/module-installation](../runner/module-installation.md)
- [project/instrukt/spec/feature/runner/single-ingress-webhook-termination](../runner/single-ingress-webhook-termination.md)
