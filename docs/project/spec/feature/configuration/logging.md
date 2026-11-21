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

instrukt logs through the shared InstruktAI logging library, following its
[logging contract](https://github.com/InstruktAI/logging/blob/main/docs/contract.md).

- The source is the top-level command that wrote it (`sync`, `docs`,
  `tick`, `blueprint`, `todo`, `repo`, …), so `instrukt-ai-logs instrukt
  --logs tick` reads only the runner's ticks and `instrukt-ai-logs
  instrukt` reads them all.

### What is logged and where it goes

- Diagnostics — the structured `event` records the observability policy asks
  for (`sdlc/policy/observability`) — go to the log, in the contract's line
  format, from the logging crate the contract ships for Rust
  (`instrukt-ai-logging`), pinned to the released crates.io version
  `=0.10.3`, including its caller-owned stderr writer and path redaction.
  instrukt does not format log lines itself. The dependency owns and tests
  locations, rotation, redaction and its own diagnostics; instrukt acceptance
  tests cover command results, selected command sources and emitted events.
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
- instrukt initialises logging before the command runs. When the file writer
  cannot open its destination, instrukt switches diagnostics to standard error,
  emits its fallback warning and continues the command. The logging crate owns
  the error description and rendering. The fallback writes INFO and above for
  instrukt and WARNING and above for third-party loggers. Stderr writes are best
  effort.

## Use cases

### UC-LG1: A developer reads what instrukt did after a command ended

```gherkin
Given a developer commits to their project's main and the hook publishes its documentation
When they later want to know what that publication did
Then the published documentation is retrievable and its publication record is associated with the command that published it
```

### UC-LG2: A developer can run a command when its log directory is unwritable

```gherkin
Given instrukt cannot open the developer's log directory
When the developer runs a command
Then the command produces its usual output and exit status
And instrukt reports switching diagnostics to standard error and records that publication completed
```

## Known caveats

- Rotation and retention belong to the logging contract and its tooling, not
  to instrukt. Arbitrary directories outside the default shared root are not
  automatically covered by the rotation setup.

## See Also

- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
- [sdlc/policy/observability](../../../../domain/sdlc/policy/observability.md)
