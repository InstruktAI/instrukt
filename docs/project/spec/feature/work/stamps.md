---
description: Evidence-bound workflow facts with accountable actors and model, version and evaluation provenance.
visibility: public
---
# Stamps — Spec

## What it is

A stamp records a workflow fact against the evidence that earned it. It makes
reviews, approvals and handoffs inspectable without turning the displayed
lifecycle position into an independently editable source of truth.

## Canonical fields

### Evidence and attribution

Every stamp carries its fact, timestamp, acting session or approving person,
target evidence, model identity, model version and eval fingerprint. A stamp's
actor and its model provenance are distinct. A GitHub approval retains the
identity GitHub reports; a human approval stated in a session is attributed
to that session’s runtime and id and recorded by its agent. No identity is
<!-- planned-change:thin-live-runtime-boundary -->
guessed. An agent session's model and version come from the active runtime adapter's
read of that session's state (on T3, the provider and model `t3 thread show`
reports for the caller's thread); when the adapter reports none, or the read
fails, the stamp records the model as explicitly unknown, never a guess. Outside an agent session, model
<!-- change:thin-live-runtime-boundary -->
guessed. An agent session's model and version come from the active runtime adapter's
live `read_state` observation (`project/instrukt/spec/feature/runtime/adapter`;
on T3 via `project/instrukt/spec/feature/runtime/t3`); when the adapter reports
none, or the read fails, the stamp records the model as explicitly unknown,
never a guess. Outside an agent session, model
<!-- /planned-change:thin-live-runtime-boundary -->
and version are explicitly absent and the actor is named. Every stamp copies the current published eval
fingerprint at write time.

An artifact-bound stamp identifies the reviewed or approved content digest.
A commit-bound stamp identifies the reviewed or handed-off commit. A review
also binds its complete report's digest, verdict, review phase and round.
Required reasons are retained with skip, hold, failure and re-derivation facts.
Project, todo, phase, role, review findings and human intervention remain
associated with the fact they describe, so evaluation can derive outcomes from
the work record.

A stamp counts only for the evidence it names. Rewriting an artifact does not
transfer approval to its replacement. Rebuilding after a review does not turn
the earlier review into a verdict on a different commit. Earlier decisions
remain available as history when current evidence changes.

### Historical preparation-head stamps

Legacy `preparation_head_reviewed` stamps remain readable on historical records
for backward compatibility; new work uses `build_review` and `--finalize-ready`
in the todo's single `<slug>/work` worktree.

### Mutation boundary

`instrukt todo stamp <slug>` records an allowed fact. The whole resulting
record is validated before its transaction in the state database commits;
the stamp, the todo's new revision and its pending reflection commit
together. An unknown stamp,
missing target, unresolved required evidence, missing required attribution or
invalid resulting state is refused without a partial local stamp.

### Decision attention and completion evidence

Approval and change-request writes create a pending attention record in the
same transaction. Attention delivery tracks the current claim and retries from
that pending record. Replaying an applied decision creates no duplicate.

A completion stamp releases the caller's open workspace or child attempt in the
same transaction. A workspace attempt requires the bound worktree and branch
at a clean head. A worktree-less attempt has no Git placement to check. An
unchanged replay returns its retained stamp. A caller without an open attempt
can record the fact without releasing anyone else's attempt. A structural
holder accepts completion or replay of its own bound attempt through
`--role-complete`.

### Stamp forms

| Command suffix after `instrukt todo stamp <slug>` | Bound evidence and result |
| --- | --- |
| `--verdict approve\|needs_work --phase prd_review\|architecture_review\|plan_review` | Current phase target and findings digest. |
| `--verdict approve\|changes_requested --phase build_review` | Exact commit and current `review-findings` digest. Approval requires an independent session. |
| `--architecture-skipped <reason>` | Applicable item and recorded reason. |
| `--diagnosis-settled` | Current diagnosis digest. |
| `--diagnosis-unresolved --reason <text>` | Current diagnosis and the unresolved reason. Establishes the dispatch hold. |
| `--drift-reviewed <artifact> cosmetic\|substantive <reason>` | Artifact, classification, reason, digest and current grounding. |
| `--intent-checked [--conversation <runtime:id>]` | Current `input` digest, checking actor and supplied conversation when present. |
| `--kickback --reason <text>` | Current input, invalidated plan and affected review evidence. |
| `--build-started` | Original base commit and current plan digest. Requires configured PRD approval where applicable. |
| `--build-complete` | Current commit. A later build-completion stamp clears recorded finalize readiness. |
| `--role-complete` | Completion, with role and attempt number when bound and Git coordinates when the bound attempt has a workspace. |
| `--deferrals-processed` | Current deferrals digest. |
| `--finalize-ready` | Exact integration candidate. Appends `workflow.finalized` in the same transaction. |
| `--verification pass\|fail\|blocked\|tier_0_skip\|hotfix_deferred [--follow-up <slug>]` | Exact commit and current `verification-findings` digest. A pass requires an independent session. |
| `--task-firing-started [--occurrence <RFC 3339>]` | Due occurrence and start. Requires configured task-plan approval. |
| `--task-outcome success\|error [--reason <text>] [--occurrence <RFC 3339>]` | Result of the caller's current execution. An error has a reason. |

Approval and intent-check attribution are specified in
`project/instrukt/spec/feature/work/approvals`. Reflection and GitHub replay are
specified in `project/instrukt/spec/feature/work/github-record`.

### Build start, completion and review rounds

A later build-start stamp preserves the original base. A workspace completion
stores the head at release, and `--role-complete` retains the commits between
the captured base and released head. Historical role completions retain their
recorded artifact references. A recorded failure releases the attempt without a
completion stamp. Review rounds count verdict stamps.

Finalize readiness requires the three integrity checks in
`project/instrukt/spec/feature/workflow/delivery-cycle`. Its stamp and
`workflow.finalized` event commit together, with event identity keyed by
project, todo and commit. The execution claim stays held.

### Verification dispositions

A disposition binds the exact commit and the current digest of
`verification-findings`. A hotfix deferral requires `--follow-up <slug>` naming
another todo in the project that is neither delivered nor withdrawn. The stamp
keeps that todo's durable identity, which survives rename. The other
dispositions take no follow-up.

The latest disposition counts for handoff when it admits handoff and both the
commit and report digest match. The raw verification evidence is outside the
stamp and is not stored by instrukt.

## Use cases

Stamp capture supports the approval, GitHub-record and evaluation-report goals.
Shared completion and release are exercised by UC-DC2, UC-DC4, UC-DC5, UC-DC6,
UC-DC7 and UC-DC10 in `project/instrukt/spec/feature/workflow/delivery-cycle`.

## Known caveats

- Role completion alone establishes neither current independent approval nor
  finalize readiness; each binds its own current evidence.

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
<!-- planned:thin-live-runtime-boundary -->
- [project/instrukt/spec/feature/runtime/adapter](../runtime/adapter.md)
<!-- /planned:thin-live-runtime-boundary -->
- [project/instrukt/spec/feature/workflow/delivery-cycle](../workflow/delivery-cycle.md)
- [project/instrukt/spec/feature/work/state-database](state-database.md)
- [project/instrukt/spec/feature/work/todos](todos.md)
