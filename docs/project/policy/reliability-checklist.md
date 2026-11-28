---
description: Diff-triggered live verification checks for instrukt born solely from confirmed real occurrences with auditable evidence, governed by sdlc/policy/live-verification.
---

# Reliability Checklist — Policy

## Required reads

- [project/instrukt/spec/verification-substrate](../spec/verification-substrate.md)

## Rules

Project fill for the `project/<project>/policy/reliability-checklist` extension
point declared by `sdlc/procedure/lifecycle/work/verify`. Semantic diff-trigger
evaluation and Promote -> Automate -> Prune maintenance follow
`sdlc/policy/live-verification`.


| Semantic Diff Trigger | Mandatory Live Check | Originating Real Occurrence |
| --- | --- | --- |
| Diff changes `instrukt.db` writer locking, `Session` lock lifetimes, or GitHub reflection flushing (`crates/instrukt-todos/src/work/`) | Against an isolated temp `INSTRUKT_HOME` with a delayed local loopback GitHub reflection stub holding a runner tick's reflection flush (`instrukt tick`) in flight, invoke a local state write (`instrukt todo create` or `instrukt todo artifacts write` via the built candidate binary) and verify via a concurrent read (`instrukt todo show <slug> --json` or `instrukt todo list`, which takes neither lock per `project/instrukt/spec/feature/work/state-database`) that the local SQLite write commits, returns immediately without touching GitHub or waiting for `reflection.lock`, and becomes visible immediately while the runner tick's reflection request remains stalled | [Issue #106](https://github.com/InstruktAI/instrukt/issues/106) (GitHub reflection held `writer.lock` across remote network I/O and starved concurrent local runner state writes to timeout) |
| Diff modifies `scripts/ci-changes.sh`, or adds or changes a Rust test read (`CARGO_MANIFEST_DIR`, `include_str!`, `include_bytes!`) in `crates/` or `tests/` of a repository file outside `crates/` | Run `scripts/ci-changes.sh --classify` on the repository paths the workspace tests read or compile, and its `pull_request` mode on a real git diff of representative paths, and verify every such path selects `rust=true` while the documentation and configuration paths for which `project/instrukt/policy/testing` runs no Rust check, such as root `instrukt.yaml`, keep `rust=false` | [Issue #191](https://github.com/InstruktAI/instrukt/issues/191) (docs-only PR #163 passed pull_request CI without the workspace tests, which read shipped `docs/domain/sdlc/*.md` and `docs/general/*.md`; main push run `45214982312` then failed) |
| Diff changes sidecar-free SQLite snapshot inspection (`crates/instrukt-core/src/state_db/_snapshot.rs`, `Store::read(true)`, `Store::stored_schema`, or `state_db::inspect_file`), including POSIX `fcntl(F_GETLK)` lock probing, Ticket #3883 re-check, empty-DB handling, or retry backoff | Against isolated temp `INSTRUKT_HOME` slots (`scripts/with-runtime-homes.py`) using the built candidate `./target/debug/instrukt` binary, exercise sidecar-free inspection (`instrukt events status --json` and `instrukt status`) against: (1) an active external SQLite rollback-journal writer holding real POSIX `F_WRLCK` within `0x40000000..+512` with a non-zero `instrukt.db-journal` — verify inspection waits until commit, returns committed state, and creates no sidecars or `writer.lock`; (2) a `SIGKILL`-crashed writer leaving a genuine non-zero hot `instrukt.db-journal` with `F_UNLCK` — verify inspection fails promptly (well below `BUSY_TIMEOUT`) with `rollback recovery is pending; inspect after the writing operation completes`, leaving `instrukt.db` and `instrukt.db-journal` bytes untouched and creating no sidecars or `writer.lock`; and (3) a 0-byte `instrukt.db` with no sidecars — verify inspection succeeds cleanly as empty state without modifying the 0-byte file or creating sidecars or `writer.lock` | [Issue #449](https://github.com/InstruktAI/instrukt/issues/449) (on PR #447, `test ubuntu (4/8)` runs [46115435280/140888497142](https://github.com/InstruktAI/instrukt/actions/runs/46115435280/job/140888497142) and [46115435280/140891678149](https://github.com/InstruktAI/instrukt/actions/runs/46115435280/job/140891678149) failed twice consecutively in `conversations::an_operator_onboards_a_company_while_the_runner_runs` with `rollback recovery is pending; inspect after the writing operation completes` while `instrukt run` initialized a fresh `state/instrukt.db` under `PRAGMA journal_mode = WAL`; fixed in PR #451) |


## Rationale

Each entry captures a failure mode that escaped static review and unit tests in
real use and pins it to an auditable source occurrence so live verification
concentrates on proven regression surfaces.

## Scope

All Tier 1 and Tier 2 candidate verifications in the `instrukt` repository.

## Enforcement

`verifier` evaluates this checklist against the candidate diff during
`sdlc/procedure/lifecycle/work/verify`. `todo-orchestrator` and `build-reviewer`
verify that every entry cites an inspectable real occurrence and obeys
`sdlc/policy/live-verification`.

## Exceptions

- Tier 0 changes skip live execution.

## See Also

- [project/instrukt/spec/verification-substrate](../spec/verification-substrate.md)
- [project/instrukt/spec/feature/work/state-database](../spec/feature/work/state-database.md)
- [sdlc/policy/live-verification](../../domain/sdlc/policy/live-verification.md)
