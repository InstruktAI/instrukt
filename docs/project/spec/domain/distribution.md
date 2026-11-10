---
description: Distribution owns tagged release publication and its assets, with binary installation and the cockpit skill outside the binary.
---
# Distribution — Spec

## What it is

<!-- planned-change:release-installer -->
The distribution context owns tagged release publication: version agreement,
per-target archives, checksums and the complete published asset set. Release
tooling is outside the instrukt binary and its modules.
<!-- change:release-installer -->
The distribution context owns how instrukt gets onto a computer and into the
agents that use it: the release a version tag publishes, the installer that
puts the binary on a computer and upgrades it, and the cockpit skill that
installs with one sentence into every agent CLI on the computer and is the
one way in. Nothing in it is part of the `instrukt` binary or one of its
modules. It adds no daemon, no plugin, no hook and no agent configuration
beyond the skill the `skills` command installs.
<!-- /planned-change:release-installer -->

## Ubiquitous language

**Release**
: The published GitHub release of a tag `v<version>`: an archive per target,
  `SHA256SUMS` and `install.sh`. Never changed once published.

**Target**
: A platform a release is built for, named by its Rust target triple.

<!-- planned:release-installer -->
**Release installer**
: `install.sh`: downloads, verifies and installs one release's binary into
  the install directory.

**Install directory**
: Where the installer writes `instrukt`: `$INSTRUKT_INSTALL_DIR`, else
  `$XDG_BIN_HOME`, else `~/.local/bin`.

**Cockpit**
: The instrukt skill, `skills/instrukt/SKILL.md`, versioned with the
  release: it reads `instrukt status`, then sets up or changes instrukt
  through an interview the agent guides.

**Skill install**
: `npx skills add InstruktAI/instrukt@v<version> --skill instrukt -g -y`:
  the `skills` command (`skills.sh`) installing the cockpit of one release
  into every agent CLI on the computer.
<!-- /planned:release-installer -->

## Ownership boundary

<!-- planned-change:release-installer -->
- **Owns:** the release workflow, tag/workspace/skill version agreement,
  packaging, complete asset-set checks, checksums and release publication.
- **Does not own:** commands of the binary, configuration, repository adoption,
  runner wake-up registration or publication of agent instruction files.
- **Published contracts:** release asset names, the `SHA256SUMS` format,
  release version agreement and immutable versioned publication.
- **Allowed dependencies:** workspace and skill version metadata for release
  validation, built target binaries, the packaged README and GitHub Releases.
- **Implementation roots:** `.github/workflows/release.yml`,
  `scripts/check-release-version.sh`, `scripts/package-release.sh`,
  `scripts/publish-release.sh`.
<!-- change:release-installer -->
- **Owns:** the release workflow and its assets; the installer's platform
  selection, verification, install directory and upgrade; the cockpit
  skill.
- **Does not own:** any command of the binary, including `instrukt status`
  (publication) and `instrukt -h`; the configuration the cockpit writes
  (configuration); adoption (repository); the wake-up registration
  `instrukt install` (runner); the global agent files sync renders
  (publication); where each agent CLI reads skills and when it loads them
  (the agent CLIs and the `skills` command).
- **Published contracts:** the release asset names and the `SHA256SUMS`
  format; `install.sh` with `--version <tag>`, `INSTRUKT_VERSION`,
  `INSTRUKT_INSTALL_DIR`; the skill name `instrukt` at
  `skills/instrukt/SKILL.md`; the skill install command.
- **Allowed dependencies:** publication's `instrukt status` report and the
  CLI's `-h`, which the cockpit names; configuration's, repository's
  and runner's commands, which the cockpit runs; GitHub Releases; the Agent
  Skills format and the `skills` command.
- **Implementation roots:** `.github/workflows/release.yml`,
  `scripts/check-release-version.sh`, `scripts/package-release.sh`,
  `scripts/publish-release.sh`, `scripts/install.sh`, `skills/instrukt/`.
<!-- /planned-change:release-installer -->

## Canonical fields

### Aggregates

#### Release

- **Root entity:** the tag `v<version>`.
- **Contains:** the archives of every supported target, `SHA256SUMS`,
  `install.sh`.
- **Invariants:** `<version>` equals the workspace version and the cockpit
  skill's `metadata.version` at the tagged commit; every archive has its line in
  `SHA256SUMS`; a published release is never changed.
- **Identifier:** the tag.
- **Lifecycle:** built and published by the release workflow on the tag's
  push; superseded, never edited, by the next release.

<!-- planned:release-installer -->
#### Installed binary

- **Root entity:** `instrukt` in the install directory.
- **Contains:** the binary of one release's target.
- **Invariants:** written only after its archive matched `SHA256SUMS` and
  the unpacked binary reported the release's version; replaced by rename,
  never in place.
- **Identifier:** the install directory.
- **Lifecycle:** written by the installer; replaced by a later run; removed
  by the developer.
<!-- /planned:release-installer -->

### Domain events emitted

None.

### Domain events consumed

None.

## Event subjects and origins

This context emits no events.

## Use cases

<!-- planned-change:release-installer -->
- **release-installer** — `project/instrukt/spec/feature/distribution/release-installer` — publish version-matched archives, checksums and the installer asset for every supported target.
<!-- change:release-installer -->
- **release-installer** — `project/instrukt/spec/feature/distribution/release-installer` — publish release assets; install, upgrade or pin the instrukt binary with one verified command.
- **agent-skill** — `project/instrukt/spec/feature/distribution/agent-skill` — install instrukt into every agent CLI with one sentence, and set it up or change it through the cockpit.
<!-- /planned-change:release-installer -->

## Governing designs and decisions

<!-- planned-change:release-installer -->
- `project/instrukt/design/architecture/overview` — the release-publication
  surface and the external systems it uses.
<!-- change:release-installer -->
- `project/instrukt/design/architecture/overview` — the install flow
  and the external surfaces it uses.
<!-- /planned-change:release-installer -->

## Context map position

<!-- planned-change:release-installer -->
- **GitHub Releases → distribution — Conformist:** release publication uses
  GitHub's release asset URLs as they are.
<!-- change:release-installer -->
- **publication → distribution — Published language:** `instrukt status`
  (and the CLI's `-h`) is the state the cockpit reads.
- **configuration, repository, runner → distribution — Published language:**
  `instrukt config`, `instrukt init` and `instrukt install` are the commands
  the cockpit runs; it writes no instrukt file itself.
- **Agent Skills and `skills` → distribution — Conformist:** the cockpit
  follows the Agent Skills format, and is installed by the `skills` command
  as published.
- **GitHub Releases → distribution — Conformist:** the release and the
  installer use GitHub's release asset URLs as they are.
<!-- /planned-change:release-installer -->

## Known caveats

None beyond those recorded in the feature specs.
