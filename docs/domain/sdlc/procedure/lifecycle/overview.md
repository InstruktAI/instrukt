---
eval: true
activation_trigger: Use when asked to orchestrate a todo, drive instrukt todo work for a slug, or learn how the prose-routed lifecycle works.
description: 'Unified prose-routed todo orchestrator playbook. Inspect factual ledger, route composable primitives by scope and blast radius, enforce three hard integrity invariants, and hand verified candidates to integrate.'
visibility: public
role: todo-orchestrator
parameters:
  - name: slug
    required: true
---

# Lifecycle Overview — Procedure

## Required reads

- [general/principle/session-lifecycle](../../../../general/principle/session-lifecycle.md)
- [general/procedure/orchestration](../../../../general/procedure/orchestration.md)
- [general/concept/agent-characteristics](../../../../general/concept/agent-characteristics.md)
- [general/policy/commits](../../../../general/policy/commits.md)
- [sdlc/policy/todo-creation-discipline](../../policy/todo-creation-discipline.md)
- [sdlc/policy/peer-invocation](../../policy/peer-invocation.md)
- [sdlc/procedure/blast-radius-discovery](../blast-radius-discovery.md)

## Goal

Drive a todo end-to-end in a single unified `todo-orchestrator` session using
one `<slug>/work` worktree and a facts-only ledger (`instrukt todo show <slug>`
/ `instrukt todo work <slug>`). Code enforces only facts and three hard
integrity invariants; intelligence routes composable primitives in prose
according to scope, blast radius, and risk — never a rigid 42-state transition
table.

### What Code Enforces (Three Hard Integrity Invariants)

1. **No interference:** one active writer/child workspace attempt per todo
   worktree at a time (`instrukt todo attempt` / `bind`, atomically released on
   the role's completion stamp).
2. **Independent exact-SHA review gate at `integrate`:** only the standing
   `integrator` writes `main`, and only for an exact commit SHA that carries an
   **independent reviewer's `build_review` `approve` stamp** bound to that exact
   SHA and current `review-findings` report + green CI. Reviewers stamp their
   own verdicts; self-approval by the orchestrator or any writer is refused.
3. **Human-owned approval gates:** configured `approvals.prd` (required for
   `feature` todos and any todo carrying a `prd` artifact before build or
   finalization) and `approvals.task` (required before `task-runner` executes a
   `task` todo).

### Composable Primitives & Prose Routing

Choose only the primitives that earn their keep for this todo's actual scope
and blast radius. Every route is reversible: if a fast-tracked build discovers
unexpected blast radius or needs a contract/spec rewrite, pause implementation
and route to `planner`, `intaker`, or doc updates cleanly in the same
`<slug>/work` worktree.

- **Bounded 5-minute bug or small localized change:**
  1. Check blast radius yourself (`sdlc/procedure/blast-radius-discovery`). For
     a bug whose root cause is non-obvious, dispatch `diagnoser`
     (`sdlc/procedure/lifecycle/prepare/diagnose-peer`) and settle the
     diagnosis first.
  2. Dispatch `builder` (`sdlc/procedure/lifecycle/work/build`) directly in
     `<slug>/work` — no mandatory `implementation-plan`, `setup`, or
     `quality-checklist` ceremony.
  3. Dispatch an independent `build-reviewer`
     (`sdlc/procedure/lifecycle/work/review-build`) against the exact clean
     committed HEAD.
  4. If `changes_requested`, dispatch `builder`/`fixer`
     (`sdlc/procedure/lifecycle/fix-review-findings`) to resolve findings and
     re-run independent `build-reviewer` on the new exact HEAD.
  5. Process any `deferrals` artifact via `deferrer`
     (`sdlc/procedure/lifecycle/work/deferrals`).
  6. Push the exact reviewed HEAD to `origin/<slug>/work`, stamp
     `instrukt todo stamp <slug> --finalize-ready` (directly as the
     claim-holding `todo-orchestrator`, or via `finalizer` when rebase/merge
     sync is needed — note that any new merge commit requires fresh exact-SHA
     independent review), then release your claim with
     `instrukt todo update <slug> --unclaim` to hand off to `integrate`.

- **Non-trivial change or high blast radius:**
  1. Dispatch `planner` (`sdlc/procedure/lifecycle/prepare/plan-draft`) for
     blast-radius discovery and `implementation-plan`, followed by independent
     `plan-reviewer` (`sdlc/procedure/lifecycle/prepare/review-plan`).
  2. Optionally dispatch `architect` + `architecture-reviewer` if structural
     boundaries or durable state change.
  3. Proceed to `builder` → independent `build-reviewer` → fix/deferrals as
     needed → exact-SHA push + `--finalize-ready` + `--unclaim` → `integrate`.

- **Feature (`kind: feature`):**
  1. Verify `input` intent (`--intent-checked` when authored by another
     session).
  2. Dispatch `intaker` (`sdlc/procedure/lifecycle/prepare/intake`) to author
     `prd` (and pending feature-spec scenarios when applicable), then
     independent `prd-reviewer`
     (`sdlc/procedure/lifecycle/prepare/review-prd`).
  3. Obtain configured PRD approval (`approvals.prd`). When human approval is
     required, present the reviewed PRD clearly, unclaim
     (`instrukt todo update <slug> --unclaim`), and stop — the runner wakes you
     automatically when the approval arrives.
  4. Dispatch optional `architect` + `architecture-reviewer` (or skip when
     unneeded), then `planner` + `plan-reviewer`.
  5. Proceed to `builder` → independent `build-reviewer` → exact-SHA push +
     `--finalize-ready` + `--unclaim` → `integrate`.

- **Task (`kind: task`):**
  Tasks never create a git worktree. Draft `task-plan` directly, or delegate to
  `task-preparer` (`sdlc/procedure/lifecycle/task/prepare-task`) as a
  worktree-less child attempt (`instrukt todo attempt` / `bind` without
  `--worktree`, `--branch`, or `--base`, completed via `--role-complete`), then
  unclaim and wait for `approvals.task` when human-owned. Once approved, the
  runner dispatches `task-runner` (`sdlc/procedure/lifecycle/task/run-task`) on
  schedule.

### Waiting & Re-Dispatch Discipline

The runner suppresses re-dispatch for unclaimed active todos only while waiting
on configured human approval (`approvals.prd` / `approvals.task`) or while a
`--finalize-ready` handoff stands for `integrate`. When waiting on anything else
(a human clarification, external input, or a peer reply), keep the claim on your
live session or place an explicit hold (`instrukt todo update <slug> --hold --reason "..."`)
so the runner does not re-dispatch a fresh turn on every tick.

## Preconditions

- `instrukt todo show <slug> [--json]` displays the factual ledger: artifacts,
  digests, stamps, claims, bindings, git state, open findings, and gate status.
- The selected flavor supplies `workflow/workflow.yaml` declaring
  `orchestrator: todo-orchestrator`, `requires`, and `roles`.

## Steps

### Workspace placement & delegation

<!-- @config: runtime.start_thread -->

<!-- @config: runtime.dispatch -->

<!-- @config: runtime.message_child -->

<!-- @config: runtime.inspect_thread -->

<!-- @config: runtime.retire_thread -->

1. **Read current facts and claim the todo.** Run `instrukt todo work {slug}`
   (or `instrukt todo show {slug} --json` to inspect). For deliberate entry
   outside `<slug>/work`, follow the returned native placement instruction and
   re-enter with `--placement-evidence`. On resumption after interruption,
   kickback, or human approval, read the factual ledger first so your next move
   matches ground truth.
2. **Check prerequisites and holds.** If prerequisites are undelivered or a
   hold is active, preserve state and stop without guessing.
3. **Select and dispatch the next primitive.** Reserve each worker attempt
   through `instrukt todo attempt {slug} --role <role> --procedure <id> --args {slug} --brief -`,
   delegate with shared isolation in `<slug>/work`, and immediately bind the
   child via `instrukt todo bind`. Only one workspace attempt may be active at
   a time; completion stamps release it atomically.
4. **Verify review substance before acting on verdicts.** Confirm the findings
   artifact exists, covers all applicable review lanes, and justifies clean
   approvals specifically for the target.
5. **Finalize and hand off to `integrate`.** Confirm the worktree is clean,
   no workspace attempt is active, `origin/<slug>/work` matches the exact
   independently approved HEAD commit SHA, and all required approvals hold.
   Record `instrukt todo stamp {slug} --finalize-ready` (which atomically emits
   `workflow.finalized`) and release the claim with
   `instrukt todo update {slug} --unclaim`. Keep this orchestrator thread
   wakeable (never retire it before delivery — `integrate` retires it once the
   todo is delivered).
6. **Retire on direct withdrawal.** When this orchestrator withdraws or removes
   a todo directly (for example under triage, retraction, or a no-fix exit),
   finish all record writes (`log`, `instrukt todo remove {slug}`) and confirm
   the removal outcome first, preserve any unfinished human or worker
   obligations, and then retire this terminal orchestrator thread via the
   configured `runtime.retire_thread` action (converging idempotently on replay
   and never retiring standing agents).
7. **Author human-directed ad-hoc process changes in isolation.** When the
   human explicitly directs an ad-hoc process change in this orchestrator
   session, author it in a separate clean worktree and branch off the canonical
   branch under `sdlc/policy/process-change-authority`, leave `<slug>/work` and
   its branch untouched, declare human presence at each commit under
   `general/policy/human-involvement-recording`, and route the commit to an
   independent no-stake commit reviewer.

## Outputs

- Verified candidate commit pushed to `origin/<slug>/work` with an independent
  exact-SHA `build_review` `approve` stamp, `--finalize-ready` handoff to
  `integrate`, and released claim — or an approved `task-plan` ready for
  `task-runner`.

## Recovery

- Re-read `instrukt todo show {slug}` after any interruption or failure; all
  state lives in the factual ledger and git worktree.
- If a child stops without stamping completion, inspect its state, reconcile
  the worktree to a clean head, and resolve the attempt with
  `instrukt todo attempt {slug} --role <role> --attempt <n> --failed ...` (or
  `--not-started` if it never started) before dispatching a replacement.
- If `integrate` returns red CI or conflicts, re-enter `<slug>/work`, resolve
  through `builder`/`fixer`, obtain a fresh independent exact-SHA review
  approval, push, and re-stamp `--finalize-ready`.

## See also

- [sdlc/procedure/lifecycle/integrate](integrate.md)
- [sdlc/procedure/lifecycle/work/build](work/build.md)
- [sdlc/procedure/lifecycle/work/review-build](work/review-build.md)
- [sdlc/procedure/lifecycle/work/finalize](work/finalize.md)
