---
source_sha256: acd7fdce8428cb2f7214953d50948ce5f5534c4b569e3d888f90b430718002ef
model: claude-opus-5-5
---
- before durable state gets a home, MUST state: nature (authoritative | derived from a named source | external bookkeeping: fingerprints, cursors, delivery marks | transient), its single writing owner, and loss cost (nothing | recomputation | network transfer | repeated external effect | lost intent).
- loss beyond cheap local recompute -> owner persists it when learned, surviving restarts, installs, crashes; process memory holds only what restarts rebuild without network or repeated effects, never the only copy.
- an external effect whose outcome must be recognised later -> durable record of intent first -- recovery completes or recognises it instead of repeating.
- a derived store states how it is rebuilt and the cost; NEVER keep other state inside it -- rebuild destroys it.
- state file = small, bounded, one writer, read whole/by key, never queried across; one document per owner and scope, written whole and atomically (write beside, rename over).
- database = unbounded growth, frequent partial updates, several writers, queries/indexes or multi-record transactions.
- write-isolation unit = scope where writers wait on each other (SQLite: one file); share one only within an owning domain and compatible write pattern; background writers (pollers, outboxes, queues, maintenance) NEVER share one with a path a person or agent waits on; a differing domain gets its own -- fewer files is no reason.
- each state names its scope (machine | project | home | cluster); state of a moving responsibility moves with it by a defined transfer.
- a growing store names its bound and retirement; current state kept apart from history/logs/outboxes -- retention prunes history, never current state.
- applies to state a delivery adds, moves or changes; an existing store is brought in line when the change path touches it.
- architecture records each piece of durable state and its review verifies it; work without reviewed architecture records it in the plan; build and commit review reject undeclared state or storage breaking these rules.
- exception: a tool with no background process whose writes all run in bounded invocations under one per-computer writer lock may share one database across domains -- the lock is its isolation unit, named in the architecture.
