---
eval: true
description: 'Build phase. Execute implementation plan tasks, commit per task, and manage deferrals.'
visibility: 'public'
role: builder
parameters:
  - name: slug
    required: true
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: med
---

# Build — Procedure

## Required reads

- @~/.instrukt/docs/sdlc/bundle.md
- @~/.instrukt/docs/sdlc/policy/quality-predicate-registry.md
- @~/.instrukt/docs/sdlc/principle/module-depth.md
- @~/.instrukt/docs/sdlc/policy/substrate-anchoring.md
- @~/.instrukt/docs/sdlc/policy/preparation-artifact-quality.md

## Goal

Execute the implementation plan for a todo and produce review-ready changes.
Candidate verification belongs to the integration gate.

## Preconditions

- `instrukt todo artifacts read {slug} implementation-plan` exists and is readable.
- The checked `input` artifact is readable for every kind. For feature work,
  the approved `prd` derives its requirements; for bug work, `diagnosis`
  sharpens the reported symptom without replacing `input` as intent.
- `architecture` (thin doc-edit log) is readable when present and not skipped; consult the domain/design snippets it points to for technical context.
- `instrukt todo artifacts read {slug} quality-checklist` exists and includes `## Build Gates (Builder)`.
- The todo scope is implementable in one build phase.

## Steps

First read the sdlc principles that apply to this work, as `instrukt docs get --domain sdlc` lists them, with `instrukt docs get`.

Apply the artifact-discovery rule from `preparation-artifact-quality`
(`instrukt todo artifacts list {slug}`) before assuming only the numbered
artifacts below exist — read `log` when present, and any other present
artifact relevant to executing this plan.

Apply the testing contract (`sdlc/policy/testing`) and the project's testing
ruling to every testing instruction below. Author or change tests only where permitted. Suite execution belongs to
the integration gate; project-owned hooks and CI retain their own checks. When no tests apply, report `N/A`
with the governing contract.

1. Read `input` to confirm the checked intent and scope constraints. For a
   feature, also read `prd`; for a bug, read `diagnosis`. Derived artifacts
   carry no authority to widen the recorded intent.
2. Read `architecture` when present and not skipped — it is the thin
   doc-edit log; consult the domain/design snippets it points to for
   technical context and intended invariants. The prepared success criteria are
   the verification contract.
3. Read the implementation-plan artifact via
   `instrukt todo artifacts read {slug} implementation-plan` and identify the tasks
   not yet delivered by commits in the worktree.
4. Work from the scenario delta. The prepared artifacts identify the feature-spec scenarios this
   todo delivers — added, changed, and removed. For each:
   - **Added scenario** — bind it: write the test at the spec-mirrored path
     and confirm it fails, implement the required behavior, then confirm it passes.
   - **Changed scenario** — update the bound test to the new contract
     and confirm it fails, update the code, then confirm it passes.
   - **Removed scenario** — delete its bound test, then the code it exclusively covered.
     You bind the scenarios the spec defines; you never invent what to test. See `policy/testing`.
5. Execute tasks sequentially:
   - Before editing, resolve the active todo worktree root with `git rev-parse --show-toplevel`. Resolve every edit target from that root and its repository-relative path. Use repository-relative paths where the edit tool supports them; when it requires an absolute path, construct it from the freshly resolved root. Do not reuse absolute paths from the project checkout or another worktree. After each edit, verify the expected target with `git -C <worktree-root> diff -- <relative-path>` before continuing.
   - Follow the build mode determined in step 4.
   - Commit changes for the task (one commit per task). The commit is the record
     that the task is done — the implementation-plan artifact is prepare-owned and
     not writable from a work-phase session.
   - Keep the flow commit-based inside the current worktree.
   - When a test or check disagrees with the current code, do the intent-resolution work before fixing either side. Ascertain what is leading for the active objective, then fix the stale side of the mismatch.
   - When adding or updating tests, assert behavioral contracts at public boundaries. Do not satisfy a task with logger assertions, log-level checks, config/model shape checks, private-helper tests, constructor/DI wiring captures, or mock-call echoes. If the plan asks for those, reinterpret it to the nearest behavior; if no behavior exists yet, surface the plan defect instead of inventing fake coverage.

   ### Snapshot acceptance

   When project-owned verification shows snapshot diffs, read each diff before
   accepting it. If the diff matches the task's stated intent, use the project
   tool's snapshot-update option declared in its `test` action and commit the
   updated snapshot files with the task. Read that command with
   `instrukt config get actions.test`; instrukt defines no snapshot-update flag. If the diff does not match intent, the code change is wrong:
   fix the code and do not update the snapshot. Concrete idioms and migration
   patterns live in `sdlc/procedure/snapshot-testing`.

6. **Build the plan.** Your tasks are your whole concern. The conditions your
   code handles are exactly the reachable conditions
   (`sdlc/concept/defect-evidence`) of the behavior the task adds or changes:
   for each one you handle, you can show what reaches it. Where a party without
   reach to the asset can drive input into that behavior, delivering the task
   also includes every control `sdlc/policy/security-awareness` requires there,
   defense in depth included.
   What lies outside your tasks goes into `deferrals`, in the entry shape
   `sdlc/procedure/lifecycle/work/deferrals` defines, and the build continues:
   - a plan task that is out of scope, as a deferred task;
   - something you noticed along the way outside your tasks, as an
     observation: what you saw, where, and how you came to see it.
7. **Plan tasks that violate intelligence-boundaries.** If a plan task asks
   you to build a classifier, scorer, judge, evaluator, or auto-fixer on
   free-text content, stop. Per `principle/intelligence-boundaries`, the
   principle outranks the plan. Surface as a blocker and report back —
   the plan needs to change. Do not faithfully execute a task that
   substitutes code for the agent's judgment. Detection patterns:
   hardcoded vocabulary tables (`_ACK_*`, `_KEYWORDS`, `_HINTS`); regex /
   word-count thresholds making semantic decisions on free text;
   `is_meaningful` / `should_emit` / `is_actionable` predicates on
   natural language; auto-fixers rewriting agent artifacts to match
   structural rubrics. The defining test: would this code be unnecessary
   if the agent's briefing said the right thing?

8. **Own the how inside frozen intent.** The approved intent, plan tasks, and
   applicable quality gates bound the delivery; the implementation choices
   inside them are yours, including file placement, helper names, and
   decomposition, which a plan anticipates rather than binds
   (`sdlc/policy/preparation-artifact-quality`). Record a departure from what
   the plan names, with its reason, in the todo `log`,
   where the build reviewer reads it. Kick back to prepare when code reality invalidates
   the contract or a reviewed decision — not whenever implementation differs
   from prose. Never revise the plan to fit what you built. A material decision
   discovered during build — one
   whose alternatives change scope, ownership, contract, or risk — is a kickback
   trigger; `log` may record the evidence and reason for the kickback, never
   authorize the decision. When your own earlier action created the problem, the
   first remedy is to remove or undo that cause; no mechanism is added to
   accommodate a state, premise, or branch you introduced. Technical uncertainty
   goes to the reviewer through evidence or to prepare through kickback; a
   question to the human about anything outside their lane
   (`general/policy/human-contract`) is itself a defect.
9. **Honor the structural ratchet on the change path.** Execute a planned
   preparatory deepening as its own commit before the behavioral change it
   prepares; never fold it into a behavioral commit. Structural debt the plan
   did not assess is handled the same way at build time
   (`definition-of-done` §2): when the deepening is behavior-preserving,
   owner-local, contract-preserving, one coherent commit, and within the cost
   cap, do it as its own commit and record it in the build report; otherwise
   record it in `deferrals` as a ratchet candidate, naming the condition that
   failed. The
   change path is never left worse than it was found.

### Test-quality self-review

Before declaring the build complete, apply every `TQ-*` predicate in the
quality-predicate registry. Under `Build Gates (Builder)` in the
`quality-checklist` artifact, record one line per ID: concrete evidence for a
triggered predicate or `N/A` with a reason. Name the production mutation each
behavioral or smoke test kills. A green suite is execution evidence, not this
self-review.

### Pre-completion checklist

1. Every implementation-plan task is delivered by a commit or explicitly recorded in `deferrals`.
2. Confirm each added or changed bound test failed before implementation and
   passes afterward under the project's testing contract (`sdlc/policy/testing`).
   Record that evidence for integration; do not add a full-suite gate here.
3. Where the project permits test authorship, every in-scope added or changed scenario is bound; removed scenarios' tests are deleted.
4. No spec-bound assertion weakened or deleted; no in-scope spec has orphan bindings (run
   `instrukt specs coverage --check` on demand to check).
5. Every `TQ-*` predicate has builder evidence or a reasoned `N/A` in
   `quality-checklist`.
6. Record applicable project quality obligations; hooks and CI own their checks.
7. User-facing verification:
   - For browser-based frontend or web UI changes, run a browser-automation smoke check in a real browser that exercises the changed path. Record the path exercised and the observed result in `quality-checklist.md` under the Build section.
   - For other user-facing changes (CLI output, TUI, interactive behavior), verify the change is observable and record what was checked and the result in `quality-checklist.md`.
   - If verification is not possible in the build environment, note the gap explicitly in `quality-checklist.md`.
8. Working tree is clean: `git status`.
   - If not clean, commit all remaining files in the worktree. Everything in the worktree belongs to the task.
9. Verify commits exist: `git log --oneline -10`.
10. Stamp explicit build completion: `instrukt todo stamp {slug} --build-complete`.

## Outputs

- Code changes implementing the plan.
- Commits for each task.
- Optional `deferrals` when a task, a ratchet candidate, or an observation lies outside the build.

### Report format

```
BUILD COMPLETE: {slug}

Tasks completed: {count}
Commits made: {count}
Tests: <bindings and integration-verification obligations, or N/A with governing contract>
Project checks: <existing evidence or pending candidate verification>

Ready for review.
```

## Recovery

- If a task fails, analyze the error and retry.
- If stuck after two attempts, stop and report the blocker.
- **Orchestrator-surfaced process failures are yours to recover.**
  - **Upstream merge conflict:** the orchestrator's merge command aborts the
    conflicting merge and returns the failure. Fetch accepted upstream main,
    reapply the prepared intent in your worktree, resolve the conflicts and
    rerun only the tests bound to the affected use cases and commit the resolution. Preserve both
    sides' intended behavior. Record the
    changed evidence and re-stamp build completion so review binds the result.
  - **Failed project checks, a dirty worktree, a missing stamp, or stale branch
    publication:** recover within the worktree and record the result; the
    orchestrator never edits code or tests. Candidate verification remains
    with integration.
  - If the facts plus this procedure are not enough to proceed, report that
    explicitly — it is a defect in this brief, and saying so is the correct outcome.
