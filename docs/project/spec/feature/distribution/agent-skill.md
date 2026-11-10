---
description: "The instrukt skill — one cockpit skill in instrukt's own repository, installed into every agent CLI on a computer with one skills command pinned to a release; it reads `instrukt status` and then adopts a repository, switches on a module or reconfigures through an interview the agent guides."
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

One hand-authored skill. Its `description`, within the specification's
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
     in their owning layer and validate. Model selection uses the configuration
     skill and user-owned `~/.instrukt/agents.yaml`, keyed by role; the skill
     discovers existing ACP instances without creating provider instances.
     The same role entries hold classes, scoped capability bundles and GitHub
     App references. GitHub setup reuses configured Apps or creates/installs the
     three identities through the configuration skill; author and reviewer bots
     stay distinct. Protected instrukt secrets, including App keys, use only
     `instrukt secrets set/generate`.
   - Anything else about instrukt: `instrukt -h` and `instrukt docs`.
3. **Show the result** by reading `instrukt status` again.

The setup is an interview the agent guides. It asks the human only what
instrukt cannot answer: which runtime to call (no code detects or chooses
one), which docs repository to adopt when the computer publishes none, the
agent preferences and git authors, which modules, and the values a module
needs. Everything instrukt can answer comes from status and `-h`, never from
the human. Layered settings and secrets use instrukt's CLI; model profiles and
role orders use the configuration skill's `agents.yaml` contract
(`project/instrukt/spec/feature/configuration/agent-selection`). The skill never rewrites
itself and keeps no state: the human's choices live in instrukt's
configuration, so the same skill is opened again for any later change.

Once the computer is set up, the global agent files sync renders
(`project/instrukt/spec/feature/publication/agent-entrypoints`) tell every
session about instrukt's documentation; the skill stays the way in for
changing instrukt.

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

## See Also

- [project/instrukt/spec/domain/distribution](../../domain/distribution.md)
- [project/instrukt/spec/feature/distribution/release-installer](release-installer.md)
- [project/instrukt/spec/feature/publication/instrukt-status](../publication/instrukt-status.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
- [project/instrukt/spec/feature/repository/project-initialization](../repository/project-initialization.md)
- [project/instrukt/spec/feature/configuration/configuration-editing](../configuration/configuration-editing.md)
