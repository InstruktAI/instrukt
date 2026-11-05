---
description: "Distribution — how instrukt reaches a computer: the versioned GitHub release, the release installer, and the Claude plugin with its cockpit skill and startup hook. Release tooling outside the binary."
delivered_by: [instrukt-first-build]
---
# Distribution — Spec

## What it is

The distribution context owns how instrukt gets onto a computer and into the
agents that use it: the release a version tag publishes, the installer that
puts the binary on a computer and upgrades it, and the Claude plugin that
installs with one sentence and gives every session one way in. Nothing in it
is part of the `instrukt` binary or one of its modules. It adds no daemon and
no agent configuration beyond what Claude Code's own plugin system installs.

## Canonical fields

### Ubiquitous language

**Release**
: The published GitHub release of a tag `v<version>`: an archive per target,
  `SHA256SUMS` and `install.sh`. Never changed once published.

**Target**
: A platform a release is built for, named by its Rust target triple.

**Release installer**
: `install.sh`: downloads, verifies and installs one release's binary into
  the install directory.

**Install directory**
: Where the installer writes `instrukt`: `$INSTRUKT_INSTALL_DIR`, else
  `$XDG_BIN_HOME`, else `~/.local/bin`.

**Marketplace**
: instrukt's Claude Code plugin catalog, `.claude-plugin/marketplace.json` at
  the repository root, named `instrukt`.

**Plugin**
: `instrukt@instrukt`, the Claude Code plugin under `plugins/claude/`,
  versioned with the release.

**Cockpit**
: The plugin's one skill: it reads `instrukt status`, then sets up or changes
  instrukt through an interview the agent guides.

**Startup hook**
: The plugin's `SessionStart` hook, telling each session that instrukt is
  installed and that the cockpit is the way in.

### Ownership boundary

- **Owns:** the release workflow and its assets; the installer's platform
  selection, verification, install directory and upgrade; the marketplace
  file; the plugin's manifest, cockpit skill, startup hook and its script.
- **Does not own:** any command of the binary, including `instrukt status`
  (publication) and `instrukt -h`; the configuration the cockpit writes
  (configuration); adoption (repository); the wake-up registration
  `instrukt install` (runner); the global agent files sync renders
  (publication); Claude Code's plugin loading, scopes and updates (Claude
  Code).
- **Published contracts:** the release asset names and the `SHA256SUMS`
  format; `install.sh` with `--version <tag>`, `INSTRUKT_VERSION`,
  `INSTRUKT_INSTALL_DIR`; the marketplace name `instrukt` and plugin id
  `instrukt@instrukt`; the install command
  `claude plugin install instrukt --marketplace InstruktAI/instrukt`.
- **Allowed dependencies:** publication's `instrukt status` report and the
  CLI's `-h`, which the cockpit and hook name; configuration's, repository's
  and runner's commands, which the cockpit runs; GitHub Releases; Claude
  Code's plugin and marketplace formats.
- **Implementation roots:** `.github/workflows/release.yml`,
  `scripts/package-release.sh`, `scripts/install.sh`,
  `.claude-plugin/marketplace.json`, `plugins/claude/`.

### Aggregates

#### Release

- **Root entity:** the tag `v<version>`.
- **Contains:** the archives of every supported target, `SHA256SUMS`,
  `install.sh`.
- **Invariants:** `<version>` equals the workspace version and the plugin
  manifest's `version` at the tagged commit; every archive has its line in
  `SHA256SUMS`; a published release is never changed.
- **Identifier:** the tag.
- **Lifecycle:** built and published by the release workflow on the tag's
  push; superseded, never edited, by the next release.

#### Installed binary

- **Root entity:** `instrukt` in the install directory.
- **Contains:** the binary of one release's target.
- **Invariants:** written only after its archive matched `SHA256SUMS` and
  the unpacked binary reported the release's version; replaced by rename,
  never in place.
- **Identifier:** the install directory.
- **Lifecycle:** written by the installer; replaced by a later run; removed
  by the developer.

### Domain events emitted

None.

### Domain events consumed

None.

### Use cases (index of feature specs)

- **release-installer** — `project/instrukt/spec/feature/distribution/release-installer` — install, upgrade or pin the instrukt binary with one verified command.
- **claude-plugin** — `project/instrukt/spec/feature/distribution/claude-plugin` — install instrukt into Claude with one sentence, learn at startup how to reach it, and set it up or change it through the cockpit.

### Governing designs and decisions (index)

- `project/instrukt/design/architecture/overview` — the install flow
  and the external surfaces it uses.

### Context map position

- **publication → distribution — Published language:** `instrukt status`
  (and the CLI's `-h`) is the state the cockpit reads and the startup hook
  points at.
- **configuration, repository, runner → distribution — Published language:**
  `instrukt config`, `instrukt init` and `instrukt install` are the commands
  the cockpit runs; it writes no instrukt file itself.
- **Claude Code → distribution — Conformist:** the plugin and marketplace
  follow Claude Code's formats as published.
- **GitHub Releases → distribution — Conformist:** the release and the
  installer use GitHub's release asset URLs as they are.

## Known caveats

None beyond those recorded in the feature specs.
