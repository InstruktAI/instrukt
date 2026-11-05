---
description: "The instrukt Claude plugin — a marketplace plugin installed with one sentence; its one cockpit skill reads `instrukt status` and then adopts a repository, switches on a module or reconfigures through an interview the agent guides; its startup hook tells every session that instrukt is installed and that the skill is the way in."
delivered_by: [instrukt-first-build]
---
# Claude Plugin — Spec

## What it is

A developer who uses Claude Code says "Claude, install instrukt" and gets
instrukt: the plugin, the binary, and one place to start. The plugin is a
Claude Code marketplace plugin published from instrukt's own repository. It
ships one skill, the cockpit, through which an agent sets instrukt up and
changes it, and one startup hook that tells every session instrukt is there.
It runs nothing in the background and adds no setting of its own: every
choice the human makes lands in instrukt's configuration, where the CLI
reads it.

The plugin follows Claude Code's plugin and marketplace formats: the plugin
manifest (`code.claude.com/docs/en/plugins-reference`), the marketplace file
(`code.claude.com/docs/en/plugins/marketplace-reference`), skills
(`code.claude.com/docs/en/skills`) and hooks
(`code.claude.com/docs/en/hooks`). It uses only the fields named below.

## Canonical fields

### Repository layout

| Path in `InstruktAI/instrukt` | Content |
| --- | --- |
| `.claude-plugin/marketplace.json` | the marketplace: `name: "instrukt"`, `owner: {name: "InstruktAI"}`, `description`, and one entry in `plugins`: `{name: "instrukt", source: "./plugins/claude", description}` |
| `plugins/claude/.claude-plugin/plugin.json` | the manifest: `name: "instrukt"`, `version`, `description`, `author: {name: "InstruktAI"}`, `homepage`, `repository`, `license: "MIT"` |
| `plugins/claude/skills/instrukt/SKILL.md` | the cockpit skill |
| `plugins/claude/hooks/hooks.json` | the startup hook |
| `plugins/claude/scripts/session-start.sh` | the script the hook runs |

- The plugin's id is `instrukt@instrukt`; its skill is invoked as
  `/instrukt:instrukt`.
- The manifest's `version` is the instrukt release it belongs to, without
  the `v`; it changes only in a release commit, so a plugin update and a
  binary release move together
  (`local/projects/instrukt/spec/feature/distribution/release-installer`).
- The plugin has no `bin/`, no MCP server, no agent, no command and no
  `userConfig`. The binary lives where the release installer puts it, on the
  developer's own `PATH`, because git hooks and the OS scheduler run it
  outside Claude too.
- `claude plugin validate --strict` passes on the repository root and on
  `plugins/claude/`.

### Install with one sentence

"Claude, install instrukt" asks the agent to run, in the developer's shell:

```sh
claude plugin install instrukt --marketplace InstruktAI/instrukt
```

That one command adds instrukt's marketplace when it is not yet registered
and installs the plugin at user scope, so it is on in every project of the
computer. The plugin loads in the next session, or in the current one after
`/reload-plugins`. Inside a session the developer may type the same
install as `/plugin install instrukt --marketplace InstruktAI/instrukt`,
which asks them to confirm the scope. instrukt's `README.md` opens with this
sentence and the command, for a human and for an agent that has to find out
how instrukt is installed. The binary is installed by the cockpit on its
first visit (below), so the developer runs no second command.

### The startup hook

`hooks/hooks.json` declares one `SessionStart` entry with matcher
`startup|clear|compact` and one `command` hook running
`"${CLAUDE_PLUGIN_ROOT}"/scripts/session-start.sh` with a `timeout` of 10
seconds. The script prints to standard output, which Claude Code adds to the
session's context:

- when it finds `instrukt` (on `PATH`, otherwise in the release installer's
  install directory): that instrukt `<version>` is installed at `<path>`;
  that the `instrukt` skill is the way in, to see instrukt's state, adopt a
  repository, switch on a module or change the configuration; and that
  `instrukt -h` lists the commands and each command has its own `-h`;
- when it does not: that the instrukt plugin is installed without the
  `instrukt` binary, and that the `instrukt` skill installs it.

The hook says only that. It runs no other instrukt command, reaches no
network, writes nothing, and stays far under Claude Code's 10,000-character
cap on hook output. A hook that fails or times out does not block the session.

### The cockpit skill

One hand-authored skill. Its `description` (with `when_to_use`, within
Claude Code's 1,536-character listing limit) invites any agent to open it
whenever the human wants instrukt installed, set up, extended or changed, or
asks what instrukt is doing on this computer. Every visit runs the same way,
whatever was done before:

1. **Read the state.** Run `instrukt status --json`
   (`local/projects/instrukt/spec/feature/publication/instrukt-status`).
   When there is no binary, install it with the release installer pinned to
   the plugin's own `version`, then read the state. When the binary is older
   than the plugin's release, say so and offer the same pinned install.
2. **Act on what the human asked**, against that state:
   - **Adopt this repository.** When status reports it adopted, say so and
     what it publishes. Otherwise run `instrukt init`, show what it wrote and
     the claims it reports, and leave the first commit to main, which starts
     publication, to the human's word.
   - **Adopt another repository.** The same, in the repository the human
     names.
   - **Switch on a module.** When status reports a feature the module needs
     unavailable on the configured runtime, name the missing templates and go
     no further than the reconfiguration that would fill them. Otherwise ask
     what the module needs and only the human can say, write it with
     `instrukt config patch`, check it with `instrukt config validate`, and
     register the wake-up with `instrukt install` when the module runs
     through the runner.
   - **Reconfigure.** Change the requested settings with
     `instrukt config patch` in the layer each belongs to, and validate.
   - Anything else about instrukt: `instrukt -h` and `instrukt docs`.
3. **Show the result** by reading `instrukt status` again.

The setup is an interview the agent guides. It asks the human only what
instrukt cannot answer: which runtime to call (no code detects or chooses
one), which docs repository to adopt when the computer publishes none, the
agent preferences and git authors, which modules, and the values a module
needs. Everything instrukt can answer comes from status and `-h`, never from
the human. Every change goes through instrukt's CLI. The skill never rewrites
itself and keeps no state: the human's choices live in instrukt's
configuration, so the same skill is opened again for any later change.

### Updates

The marketplace is a third-party marketplace, whose auto-update is off by
default. `claude plugin update instrukt@instrukt` installs the plugin of the
newest release; its cockpit then offers the matching binary. The plugin
never upgrades the binary on its own.

### Use cases

#### UC-CP1: A developer installs instrukt into Claude with one sentence

```gherkin
Given a developer uses Claude Code on a supported computer without instrukt
When they tell Claude "install instrukt"
Then Claude installs the instrukt plugin from instrukt's marketplace for every project on the computer
And in their next session Claude learns that the instrukt skill is the way in
And when it opens the skill, the instrukt binary of the plugin's release is installed and Claude reports instrukt's state
```

#### UC-CP2: An agent learns at startup that instrukt is installed and how to reach it

```gherkin
Given a developer has the instrukt plugin and the instrukt binary installed
When they start a Claude Code session, clear it, or it is compacted
Then the agent knows instrukt is installed and which version
And that the instrukt skill is the way in
And that `instrukt -h` lists the commands
```

#### UC-CP3: A developer adopts a repository through an interview the agent guides

```gherkin
Given a developer has instrukt installed and a repository that is not adopted
When they ask Claude to adopt the repository
Then Claude reads instrukt's state and asks them only what instrukt cannot answer
And instrukt adopts the repository, and Claude shows what was written and what the repository will claim on the computer
And nothing is committed until the developer says so
When they ask the same for a repository that is already adopted
Then Claude says it is adopted and what it publishes, and changes nothing
```

#### UC-CP4: A developer switches on a module the runtime can carry

```gherkin
Given a developer's computer is configured with a runtime
When they ask Claude to switch on standing agents
And the runtime fills every template standing agents need
Then Claude asks only what the module needs from them and instrukt switches it on
And the state Claude reports afterwards shows the module on
But when the runtime leaves a needed template empty
Then Claude names the empty template, and the module stays off
```

#### UC-CP5: A developer changes their setup later through the same skill

```gherkin
Given a developer set instrukt up through the instrukt skill some time ago
When they ask Claude to change which runtime instrukt calls
Then Claude reads the current state, asks for the new choice and changes instrukt's configuration
And the skill itself is unchanged, and opening it again shows the new state
```

## Known caveats

- The shell install command needs Claude Code v2.1.292 or later; the
  in-session form needs v2.1.275 or later.
- "install instrukt" by bare name, without the `InstruktAI/instrukt` source,
  resolves only once instrukt is listed in a marketplace Claude Code already
  has registered; until then the agent learns the source from instrukt's
  README.
- Claude Code cloud sessions do not load plugins installed on the
  developer's computer.

## See Also

- local/projects/instrukt/spec/domain/distribution
- local/projects/instrukt/spec/feature/distribution/release-installer
- local/projects/instrukt/spec/feature/publication/instrukt-status
- local/projects/instrukt/spec/feature/repository/project-initialization
- local/projects/instrukt/spec/feature/configuration/configuration-editing
