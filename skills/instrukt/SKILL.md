---
name: instrukt
description: Install, set up, extend or change instrukt, or inspect what instrukt is doing on this computer, including before it is installed or configured. Read current state first; instrukt -h lists commands.
license: MIT
compatibility: Requires the instrukt binary (installed below when absent) and a POSIX shell; skill installation uses npx.
metadata:
  version: "0.1.0"
---

# Instrukt cockpit

Use this skill's `metadata.version` above for `<version>` in commands and source
URLs below. Start every visit with `instrukt status --json`. If the binary is
absent, install this skill's release, then read the state:

```sh
curl -fsSL https://github.com/InstruktAI/instrukt/releases/download/v<version>/install.sh | sh -s -- --version v<version>
```

When the binary is older than this skill's release, report the difference and
offer that pinned installer; upgrade after the human accepts. Use `instrukt -h`
and the relevant command's help to discover its current surface. If a required
command is unavailable, report that prerequisite and keep the unfinished part pending.

Guide the human's request from the observed state:

- **Adopt a repository:** work in the repository they name. If adopted, report
  what it publishes. Otherwise run `instrukt init` and show the files and claims.
  If init retained `.instrukt/adopted/AGENTS.md`, review it with the human:
  move its purpose into the master's `## Purpose` and its instructions into
  project snippets with index entries, using `instrukt docs schema docs/project`.
  Remove the kept copy after that review. The first commit to main starts
  publication; leave that commit pending until the human directs it.
- **Enable a module:** for todos, establish from status that the project's code
  is on GitHub. Establish the chosen runtime's required adapter capabilities;
  report absent prerequisites and keep the module off until they are met.
  Ask only for module values the computer cannot supply. Write the owning layer
  with revision-checked `instrukt config patch`, validate, then run
  `instrukt install`, `instrukt install --check` and relevant `ticks status`
  or `events status` for runner modules.
- **Reconfigure:** read current settings and revision, patch the owning layer,
  and validate. For model profiles, role orders/classes/capabilities and GitHub
  Apps, read the configuration skill of this release at
  `https://raw.githubusercontent.com/InstruktAI/instrukt/v<version>/skills/instrukt-configure/SKILL.md`.
  It discovers existing ACP instances and guides provider login. Keep provider
  creation with T3 and provider keys with their harness. Reuse configured GitHub
  Apps or guide creation/installation of the distinct project, author and
  reviewer identities. Store instrukt secrets through `instrukt secrets
  set/generate` in the protected machine-local secrets file.
- **Other requests:** use `instrukt -h` and `instrukt docs` to find the owning
  command or contract.

Ask for human-owned choices: runtime, which documentation repository to adopt
when none publishes, agent preferences and git authors, modules, and required
module values. Obtain discoverable machine facts through status and help.
Configuration remains in its owning files and is changed through its CLI;
model profiles and role orders follow the configuration skill's agents.yaml
contract. Keep this skill stateless and unchanged during setup.

Finish by reading `instrukt status` and report the result and any unmet
prerequisites. To install this cockpit for the computer's agent CLIs:

```sh
npx skills add InstruktAI/instrukt@v<version> --skill instrukt -g -y
```

An agent CLI loads an installed skill in its next session. The release installer
prints the matching pinned skill command; installing another release's skill
updates the cockpit, whose next visit offers the matching binary.
