---
source_sha256: acd7fdce8428cb2f7214953d50948ce5f5534c4b569e3d888f90b430718002ef
model: claude-opus-5-5
---
- MUST state, before durable state gets a home, its nature (authoritative | derived from a named source | external bookkeeping: fingerprints, cursors, delivery marks | transient), its one writing owner and its loss cost (nothing | recomputation | network transfer | repeated external effect | lost intent) -- the home follows from them.
- MUST have the owner persist state whose loss forces more than cheap local recomputation when learned, surviving restarts, installs and crashes; process memory holds only what start rebuilds without network or repeated effects, never the only copy -- restarts otherwise cost traffic and duplicated actions.
- MUST record the intent of an external effect durably before it when its outcome must be recognised -- recovery then completes or recognises it instead of repeating.
- MUST state how a derived store is rebuilt and the cost, network included; NEVER keep state of another nature inside it -- rebuilding destroys it.
- state file = small, bounded, one writer, read whole or by key, never queried across; one document per owner and scope, written whole and atomically (beside the previous, renamed over).
- database = grows without fixed bound, frequent partial updates, several writing processes, queries/indexes, or several records changed in one transaction.
- write-isolation unit = scope in which writers wait on each other (SQLite: one database file).
- MUST share a write-isolation unit only within one owning domain with a compatible write pattern; NEVER let background writers (pollers, outboxes, delivery queues, maintenance) share one with a path a person or agent waits on; a domain whose write pattern or lifecycle differs gets its own -- fewer files is no reason; shared waits add latency.
- MUST name each state's scope (machine | project | home | cluster); state of a responsibility moving between computers moves with it by a defined transfer -- else the new owner rebuilds it.
- MUST give a growing store its bound and retirement, keeping current state apart from history, logs and outboxes -- retention prunes history, never current state.
- applies to durable state a delivery introduces, relocates or changes; an existing store is brought in line when the change path touches it or by work for that purpose.
- enforcement = architecture records each piece of durable state and its review verifies it; work without reviewed architecture records it in the plan; build and commit review reject undeclared state or storage breaking these rules.
- MAY, for a tool with no background process whose every write runs in a bounded invocation (a command, or a scheduled run that exits) under one per-computer writer lock, share one database across its domains; the architecture names the lock as the unit and the invocations that hold it -- the lock already serializes all writers.
