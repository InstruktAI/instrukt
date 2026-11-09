---
description: Instrukt's upstream-watch focus — the T3 contracts, persistence migrations and CLI consumed by its runtime adapter, history and conversation links.
visibility: public
---

# Upstream Watch Focus — Spec

## What it is

Instrukt's fill of the project-focus extension point declared by
`general/procedure/upstream-watch`. It directs assessment of `pingdotgg/t3code`
changes against instrukt's runtime-adapter contracts.

## Canonical fields

Paths in this table belong to the upstream `pingdotgg/t3code` repository;
they are source paths, not instrukt snippet identities.

| Upstream surface | Instrukt contracts to check |
| --- | --- |
| Contracts package, `packages/contracts/` | Runtime operations, response shapes and capability evidence in `project/instrukt/spec/feature/runner/session-continuity`. |
| Server persistence migrations, `apps/server/src/persistence/Migrations.ts` and `apps/server/src/persistence/Migrations/`, and their projection writer, `apps/server/src/orchestration-v2/ProjectionStore.ts` | Runtime-owned conversation-source compatibility in `project/instrukt/spec/feature/runner/session-continuity`, extraction in `project/instrukt/spec/feature/runner/history`, and finished-turn relay in `project/instrukt/spec/feature/runner/conversation-links`. |
| CLI entry and commands, `apps/server/scripts/cli.ts` and `apps/server/src/cli/` | Unattended runtime operations and evidence in `project/instrukt/spec/feature/runner/session-continuity`, and native input delivery in `project/instrukt/spec/feature/runner/conversation-links`. |

Assess changes to those consumed surfaces against the owning instrukt specs;
the specs decide the required behavior. Include related upstream changes when
they affect a consumed surface.

## Known caveats

This focus supplies assessment context. Watch-source configuration is owned by
`project/instrukt/spec/feature/runner/event-activation`; listing this snippet
does not configure a watch.

## See Also

- [general/procedure/upstream-watch](../../general/procedure/upstream-watch.md)
- [project/instrukt/spec/feature/runner/session-continuity](feature/runner/session-continuity.md)
- [project/instrukt/spec/feature/runner/history](feature/runner/history.md)
- [project/instrukt/spec/feature/runner/conversation-links](feature/runner/conversation-links.md)
- [project/instrukt/spec/feature/runner/event-activation](feature/runner/event-activation.md)
- https://github.com/pingdotgg/t3code — upstream source repository.
