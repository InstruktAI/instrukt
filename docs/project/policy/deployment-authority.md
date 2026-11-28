---
description: CI is the sole builder of release-marked instrukt binaries and machine self-update is the sole deployer; agents never manually install into ~/.local/bin/instrukt or restart host services.
---

# Deployment Authority — Policy

## Rules

- Agents MUST NEVER manually build and install `instrukt` into
  `~/.local/bin/instrukt` (or any live install prefix).
- Agents MUST NEVER restart or reload host-managed `instrukt` services directly
  (`launchctl`, `systemctl`, `ai.instrukt.runner`, or owned receiver, Redis,
  wake-up, and backup timer units).
- Agents MUST NEVER compile with `INSTRUKT_RELEASE_BUILD=1` outside explicit
  human-directed recovery.
- GitHub Actions release automation (`.github/workflows/release.yml` and the
  edge release channel) is the sole builder of release-marked binaries
  (`INSTRUKT_RELEASE_BUILD=1` on the release profile).
- Machine self-update (`machine-channel-self-update`) is the sole deployer of
  installed `instrukt` binaries on live machines, relying on `instrukt install`
  as the explicit live-home backup, schema-migration, and service-restart
  boundary.
- Local development builds (`cargo build`, `cargo test`, `cargo run`, and local
  `cargo build --release`) compile without release build identity,
  unconditionally refuse `$HOME/.instrukt`, and automatically fence writes to
  `$HOME/.instrukt` and `$HOME/.local/bin/instrukt`. Local verification uses
  `scripts/dev-instrukt` or an isolated `INSTRUKT_HOME`.

## Rationale

An ad-hoc build copied into `~/.local/bin/instrukt` or pointed at
`$HOME/.instrukt` bypasses review, CI verification, pre-migration backup, and
channel deployment. A single unreviewed local build running against a live home
can silently mutate `~/.instrukt/state/instrukt.db` or owned configuration to an
unreleased schema, leaving the installed binary unable to read the machine's
own work record and services. Separating compile-time release identity,
explicit `instrukt install` migration with pre-migration backup, and CI/channel
deployment authority keeps development builds away from live operational state.

## Scope

Every agent session, developer workflow, and script operating in this
repository or on any machine running `instrukt`.

## Enforcement

- **Compile-time and runtime fence:** non-release builds unconditionally refuse
  `$HOME/.instrukt` in `Home::from_env()` with no runtime opt-out, and
  automatically fence `$HOME/.instrukt` and `$HOME/.local/bin/instrukt` at
  process startup. Compiling with `INSTRUKT_RELEASE_BUILD=1` on a non-release
  profile fails at compile time.
- **Review:** reviewers reject agent instructions and development tooling that
  manually replace the installed binary or restart host services outside the
  install/self-update boundary. They reject release markers outside CI release
  automation and runtime bypasses around the live-home fence.

## Exceptions

- Explicit human-directed emergency recovery on a broken host, authorized
  directly by the human operator for that specific incident and never encoded
  into automated agent procedures or repository scripts.

## See Also

- [general/policy/commits](../../general/policy/commits.md)
- [project/instrukt/spec/feature/runner/module-installation](../spec/feature/runner/module-installation.md)
- [project/instrukt/spec/feature/configuration/owned-files](../spec/feature/configuration/owned-files.md)
- [project/instrukt/spec/feature/work/state-database](../spec/feature/work/state-database.md)
