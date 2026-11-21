---
description: instrukt logs through the InstruktAI logging contract — one file per source in the selected app directory, readable with `instrukt-ai-logs instrukt` — so a developer or an agent can read what instrukt did after the fact.
---
# Logging — Spec

## What it is

Every instrukt command records its diagnostics where the InstruktAI logging
contract puts every InstruktAI tool's logs, so they can be read afterwards,
with the same reader, without knowing which command produced them. Hook runs,
runner ticks and blueprint workers start and end like any command, and their
diagnostics would otherwise be gone with them.

## Canonical fields

### Location and reader

- By default, the log of the app `instrukt` lives under
  `$XDG_STATE_HOME/instrukt-ai/instrukt/` (fallback `~/.local/state`, per the
  XDG Base Directory specification), one file per source:
  `<source>.log`.
- The source is the top-level command that wrote it (`sync`, `docs`,
  `tick`, `blueprint`, `todo`, `repo`, …), so `instrukt-ai-logs instrukt
  --logs tick` reads only the runner's ticks and `instrukt-ai-logs
  instrukt` reads them all.
- A nonempty `INSTRUKT_LOG_DIR` selects the complete app directory through
  the logging library: `/work/logs` places source `sync` at `/work/logs/sync.log`.
  Unset or empty keeps the XDG/default-home location above. The library's path
  normalization and tilde expansion apply. All sources share that directory;
  the reader must receive the same variable as the writer.
- Tests set `INSTRUKT_LOG_DIR` to a per-test directory, without relocating logs
  through `HOME` or an XDG base. instrukt adds no location parser of its own.

### What is logged and where it goes

- Diagnostics — the structured `event` records the observability policy asks
  for (`sdlc/policy/observability`) — go to the log, in the contract's line
  format, from the logging crate the contract ships for Rust
  (`instrukt-ai-logging`), pinned to the released crates.io version
  `=0.10.3`, including its caller-owned stderr writer and path redaction.
  instrukt does not format log lines itself. The same layer redacts event
  messages and fields before writing to either destination. The crate also
  redacts paths in its own write-failure diagnostic.
- What a command reports to its caller stays on standard output and standard
  error as each command's spec says; a refusal or failure is still printed
  to the caller, and is also logged.
- Levels are set with the contract's per-app variables, prefixed `INSTRUKT_`
  (`INSTRUKT_LOG_LEVEL`, `INSTRUKT_THIRD_PARTY_LOG_LEVEL`,
  `INSTRUKT_THIRD_PARTY_LOGGERS`, `INSTRUKT_MUTED_LOGGERS`); the default level
  is the contract's production level. The `INSTRUKT_` level variables behave
  as the InstruktAI logging contract defines them, implemented by the crate;
  instrukt adds no validation of its own. One command departs from them:
  `instrukt mcp` writes no third-party record below WARNING and gives no
  third-party logger a spotlight, whatever these variables say
  (`project/instrukt/spec/feature/runner/mcp-access`,
  `sdlc/policy/security-awareness`).
- The logging crate owns open and write failures. instrukt initialises it
  before the command runs. When the file writer cannot open its destination,
  the command continues and standard error carries the failure followed by
  fallback diagnostics. The fallback writes INFO and above for instrukt and
  WARNING and above for third-party loggers. Stderr writes are best effort.

## Use cases

### UC-LG1: A developer reads what instrukt did after a command ended

```gherkin
Given a developer commits to their project's main and the hook publishes its documentation
When they later want to know what that publication did
Then the published documentation is retrievable and the publication's record sits in instrukt's log under their own state directory, in the file of the command that wrote it, where the InstruktAI log reader finds it
```

### UC-LG2: A developer can run a command when its log directory is unwritable

```gherkin
Given instrukt cannot open the developer's log directory
When the developer runs a command
Then the command produces its usual output and exit status
And standard error reports the file logging failure and carries the command's diagnostics with the same credential redaction as the file
But if an active log file becomes unwritable during a run
Then the write-failure diagnostic on standard error redacts credentials in the log path
```

## Known caveats

- Rotation and retention belong to the logging contract and its tooling, not
  to instrukt. Arbitrary directories outside the default shared root are not
  automatically covered by the rotation setup.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [sdlc/policy/observability](../../../../domain/sdlc/policy/observability.md)
