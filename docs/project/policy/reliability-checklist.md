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
| Diff changes `instrukt.db` writer locking, `Session` lock lifetimes, or GitHub reflection flushing (`crates/instrukt-todos/src/work/`) | Against an isolated temp `INSTRUKT_HOME` with a delayed local loopback GitHub reflection stub holding a first command's reflection flush in flight, invoke an unrelated local state write (such as `instrukt todo create` or `instrukt todo artifacts write` on another slug via the built candidate binary) and verify via a concurrent read (`instrukt todo show <other-slug> --json` or `instrukt todo list`, which takes neither lock per `project/instrukt/spec/feature/work/state-database`) that the unrelated local SQLite transaction commits and becomes visible immediately while the first reflection request remains stalled — observing local commit completion separately from the writing command's subsequent reflection flush and CLI exit | [Issue #106](https://github.com/InstruktAI/instrukt/issues/106) (GitHub reflection held `writer.lock` across remote network I/O and starved concurrent local runner state writes to timeout) |
| Diff modifies `scripts/ci-changes.sh`, or adds or changes a Rust test read (`CARGO_MANIFEST_DIR`, `include_str!`, `include_bytes!`) in `crates/` or `tests/` of a repository file outside `crates/` | Run `scripts/ci-changes.sh --classify` on the repository paths the workspace tests read or compile, and its `pull_request` mode on a real git diff of representative paths, and verify every such path selects `rust=true` while the documentation and configuration paths for which `project/instrukt/policy/testing` runs no Rust check, such as root `instrukt.yaml`, keep `rust=false` | [Issue #191](https://github.com/InstruktAI/instrukt/issues/191) (docs-only PR #163 passed pull_request CI without the workspace tests, which read shipped `docs/domain/sdlc/*.md` and `docs/general/*.md`; main push run `45214982312` then failed) |


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

- Tier 0 changes with no runtime or executable-doctrine delta skip live
  execution.

## See Also

- [project/instrukt/spec/verification-substrate](../spec/verification-substrate.md)
- [project/instrukt/spec/feature/work/state-database](../spec/feature/work/state-database.md)
- [sdlc/policy/live-verification](../../domain/sdlc/policy/live-verification.md)
