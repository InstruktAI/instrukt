---
description: instrukt logs through the InstruktAI logging contract — one file per source under `$XDG_STATE_HOME/instrukt-ai/instrukt/`, readable with `instrukt-ai-logs instrukt` — so a developer or an agent can read what instrukt did after the fact.
---
# Logging — Spec

## What it is

Every instrukt command records its diagnostics where the InstruktAI logging
contract puts every InstruktAI tool's logs, so they can be read afterwards,
with the same reader, without knowing which command produced them. instrukt
has no daemon: hook runs, runner ticks and blueprint workers start and end
like any command, and their diagnostics would otherwise be gone with them.

## Canonical fields

### Location and reader

- The log of the app `instrukt` lives under
  `$XDG_STATE_HOME/instrukt-ai/instrukt/` (fallback `~/.local/state`, per the
  XDG Base Directory specification), one file per source:
  `<source>.log`.
- The source is the top-level command that wrote it (`sync`, `docs`,
  `tick`, `blueprint`, `todo`, `repo`, …), so `instrukt-ai-logs instrukt
  --logs tick` reads only the runner's ticks and `instrukt-ai-logs
  instrukt` reads them all.
- The location is a fixed rule of the contract; instrukt adds no override of
  its own. A test redirects it the way it redirects the home: through `HOME`
  (and `XDG_STATE_HOME` left unset).

### What is logged and where it goes

- Diagnostics — the structured `event` records the observability policy asks
  for (`sdlc/policy/observability`) — go to the log, in the contract's line
  format, from the logging crate the contract ships for Rust
  (`instrukt-ai-logging`), pinned to the released crates.io version
  `=0.10.0`. instrukt does not format log lines itself.
- What a command reports to its caller stays on standard output and standard
  error as each command's spec says; a refusal or failure is still printed
  to the caller, and is also logged.
- Levels are set with the contract's per-app variables, prefixed `INSTRUKT_`
  (`INSTRUKT_LOG_LEVEL`, `INSTRUKT_THIRD_PARTY_LOG_LEVEL`,
  `INSTRUKT_THIRD_PARTY_LOGGERS`, `INSTRUKT_MUTED_LOGGERS`); the default level
  is the contract's production level. The `INSTRUKT_` level variables behave
  as the InstruktAI logging contract defines them, implemented by the crate;
  instrukt adds no validation of its own.
- The logging crate owns open and write failures. instrukt initialises it
  before any other work and never continues when initialisation fails;
  there is no degraded mode.

## Use cases

### UC-LG1: A developer reads what instrukt did after a command ended

```gherkin
Given a developer commits to their project's main and the hook publishes its documentation
When they later want to know what that publication did
Then the published documentation is retrievable and the publication's record sits in instrukt's log under their own state directory, in the file of the command that wrote it, where the InstruktAI log reader finds it
```

## Known caveats

- Rotation and retention belong to the logging contract and its tooling, not
  to instrukt.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [sdlc/policy/observability](../../../../domain/sdlc/policy/observability.md)
