---
description: instrukt logs through the InstruktAI logging contract — one file per source under `$XDG_STATE_HOME/instrukt-ai/instrukt/`, readable with `instrukt-ai-logs instrukt` — so a developer or an agent can read what instrukt did after the fact.
delivered_by: [instrukt-first-build]
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
  `runner`, `blueprint`, `todo`, `repo`, …), so `instrukt-ai-logs instrukt
  --logs runner` reads only the runner's ticks and `instrukt-ai-logs
  instrukt` reads them all.
- The location is a fixed rule of the contract; instrukt adds no override of
  its own. A test redirects it the way it redirects the home: through `HOME`
  (and `XDG_STATE_HOME` left unset).

### What is logged and where it goes

- Diagnostics — the structured `event` records the observability policy asks
  for (`sdlc/policy/observability`) — go to the log, in the contract's line
  format, from the logging crate the contract ships for Rust
  (`instrukt-ai-logging`). instrukt does not format log lines itself.
- What a command reports to its caller stays on standard output and standard
  error as each command's spec says; a refusal or failure is still printed
  to the caller, and is also logged.
- Levels are set with the contract's per-app variables, prefixed `INSTRUKT_`
  (`INSTRUKT_LOG_LEVEL`, `INSTRUKT_THIRD_PARTY_LOG_LEVEL`,
  `INSTRUKT_THIRD_PARTY_LOGGERS`, `INSTRUKT_MUTED_LOGGERS`); the default level
  is the contract's production level. An invalid value is reported to the
  caller, never silently replaced.
- A log that cannot be written (an unwritable state directory) does not fail
  the command; the command reports once on standard error that its
  diagnostics could not be recorded, naming the path.

### Use cases

#### UC-LG1: A developer reads what instrukt did after a command ended

```gherkin
Given a developer's computer ran instrukt commands, including a sync triggered by a git hook
When they read instrukt's logs with the InstruktAI log reader
Then they find each command's diagnostics under their own state directory, one file per command that wrote them
And a command whose log cannot be written still completes and says its diagnostics were not recorded
```

## Known caveats

- Rotation and retention belong to the logging contract and its tooling, not
  to instrukt.

## See Also

- local/projects/instrukt/spec/domain/configuration
- sdlc/policy/observability
