---
description: 'Integrate verified candidates (with independent exact-SHA review approval where required by `general/policy/commits`), restore a red main before repair, and assess failed dependency updates.'
visibility: 'public'
role: integrator
parameters:
  - name: slug
subscriptions:
  - workflow.finalized
  - runner.ci_completed
  - runner.main_failing
  - runner.dependency_failing
subscription_home: project
interruptible: false
---

# Integration — Procedure

## Required reads

- [general/policy/commits](../../../../general/policy/commits.md)
- [general/principle/session-lifecycle](../../../../general/principle/session-lifecycle.md)
- [general/procedure/event-handling/overview](../../../../general/procedure/event-handling/overview.md)

## Goal

Merge finalized work and ready one-off worktree heads into a verified candidate,
advance canonical main to exactly that commit, recover a red baseline
before repairing its cause, and assess failed dependency updates for bug work.
The standing integrator owns composition,
failure interpretation, promotion and recovery.

## Preconditions

- Main names the repository's canonical default branch; use its actual ref.
- `workflow.finalized` identifies a todo and finalized commit, or
  `runner.ci_completed` identifies the submitted candidate and result, or
  `runner.main_failing` identifies remote main's head commit whose CI completed red,
  or `runner.dependency_failing` identifies a dependency branch, commit and failed check run for assessment.
- One standing integrator session is recorded per subscribing procedure and
  project; the runner starts or wakes that session.
- The integrator operates in the persistent integration worktree at
  `trees/_integration/`, separately from the project checkout and todo worktrees.
- Every todo candidate — and every ad hoc candidate that owes independent review
  under `general/policy/commits` — MUST carry an independent reviewer's `APPROVE`
  stamp (or digest-bound review report for ad hoc branches) bound to that
  **exact commit SHA** plus green CI before advancing `main`. An ad hoc candidate
  exempt from review under `general/policy/commits` advances on green CI alone.
  Only `integrate` writes `main`.
- Project CI and required checks, or the documented project test command when CI is
  absent, provide verification. GitHub branch protection guards main where CI exists.

## Steps

### Configured runtime operations

Apply these configured action values to the identified assignment and session.

<!-- @config: runtime.dispatch -->

<!-- @config: runtime.message_child -->

<!-- @config: runtime.read_thread -->

<!-- @config: runtime.inspect_thread -->

<!-- @config: runtime.finish_thread -->

### 1. Route the activating event

- **`workflow.finalized`:** read the todo's factual ledger, independent
  exact-SHA `build_review` `approve` stamp, and matching finalized commit.
  Reconcile the handoff against repository state. A replay of the same project,
  todo and commit is the same logical handoff.
- **`runner.ci_completed`:** reconcile the result with the exact submitted
  candidate and current GitHub check state. Proceed under Step 3 only when
  the required result is complete for that commit.
- **`runner.dependency_failing`:** inspect the exact dependency branch, commit and
  failed check run. Apply `sdlc/policy/todo-creation-discipline` to judge whether
  the evidence warrants a bug; check existing work and file kind `bug` once when
  warranted, or add the evidence to its existing owner. Preserve the failed
  dependency PR for remediation. This event is not evidence of red main and
  invokes no main revert. The project supplies its dependency-update branch
  selection to the runner's CI observation; this procedure owns the assessment.
- **`runner.main_failing`:** take the recovery path under Step 5: revert the
  offending delivery first, then repair it in its own todo.

Event delivery retains idempotency keys and per-consumer cursors. A wake-up
carries a reason to inspect current evidence, not authority to repeat a merge
or promote whatever commit now occupies the integration branch.

### 2. Compose and publish the candidate

1. Fetch current remote main and candidate branches. Inspect the integration
   worktree before reuse; preserve any unique edits or an in-flight candidate.
2. Compose from accepted upstream main in the integration worktree. Merge the
   finalized `<slug>/work` branch or ready one-off branch.
   Where `general/policy/commits` requires independent review, match independent
   approval to that exact branch head and complete report; a changed or
   unreviewed head waits for current review, and no per-commit git-note verdict
   admits a candidate. Ad hoc branches exempt under `general/policy/commits`
   require no review report.
   Uncommitted project-root files are outside the candidate and remain untouched.
3. Read the delivery context, diff and branch history. Where integration needs
   a commit, compose it around the delivery intent and record it through
   `instrukt repo commit` under `general/policy/commits`.
4. Fold pending documentation at delivery: run `instrukt docs fold <slug>` to
   reconcile the todo's pending documentation (clearing `binding_reservations`
   once their attached tests are present) under the owning documentation
   contract before freezing the candidate. Where the reviewed delivery
   introduces new project extension-point fills whose `delivered_by` markers
   this fold clears, add those newly delivered fill IDs under `listed` in
   `docs/project/index.yaml` when not yet listed, regenerate entrypoints with
   `instrukt docs render`, and verify with `instrukt docs validate` and
   `instrukt docs render --check`. Include the folded snippets, updated
   `docs/project/index.yaml`, and regenerated entrypoints in the exact commit
   verified.
5. The integrator may batch finalized candidates according to its judgment.
   Retain the work and commits represented by the batch so a failure can be
   attributed to its responsible todo.
6. Push the resulting exact commit to `origin/integrate`. Keep main unchanged
   while verification is pending. A rejected candidate push is a failed
   publication, never verification evidence.

### 3. Verify the exact candidate

With CI, the push to `integrate` runs checks. The runner checks that candidate's
CI status with conditional requests on ticks and wakes this session through
`runner.ci_completed`. Yield to that event rather than constructing a local
watch. Missing, incomplete or mismatched results leave main unchanged.

Without CI, the runner wakes this session through `runner.ci_completed` with
the result `no_ci` for the exact candidate. Read the project's documented test
command and run it in the integration worktree against that exact candidate.
A missing documented check or failing command supplies no admission. This is the local gate;
builders, reviewers and the evaluator add no suite execution of their own.

- **Green:** fast-forward main to exactly the verified candidate and push it.
  Create no additional merge commit during promotion. Confirm the remote main
  points to that commit. GitHub's required checks guard that push.
- **Red:** read the failure. Return it to the responsible todo as findings and
  record `instrukt todo stamp <slug> --kickback --reason "<failure summary>"`
  (which resets the finalized handoff so the runner wakes `todo-orchestrator`
  even when no new commit was pushed yet; note that `--kickback` also marks any
  existing `plan_review` invalidated in the ledger, so `todo-orchestrator`
  inspects the recorded reason and routes directly to `builder`/`fixer` for
  code-only or CI failures rather than re-planning unless the contract or plan
  itself is defective), make a trivial fix within integration scope, or isolate
  the culprit among a batch. A change to intent or the prepared contract returns
  through `todo-orchestrator`; a trivial integration correction is no substitute
  for it. Any changed candidate is pushed to `integrate` and verified again.

Work branches and one-off heads receive the same candidate verification.
A change to the candidate after a green
result invalidates promotion evidence for the resulting commit.

### 4. Handle conflicts and rejected promotion

**Cardinal rule: preserve the best of both worlds.** Conflicts mean two branches
independently evolved the same files. Neither side is "the winner." Both contain
intentional work that must survive. Picking one side wholesale destroys the other's
contributions — this is the leading cause of silent regression in multi-agent workflows.

**Resolution procedure:**

1. Read each conflicted file; understand the **intent** on both sides.
   - What feature, fix, or architectural change does each side introduce?
   - Are the changes orthogonal (touching different concerns in the same file) or
     genuinely contradictory (mutually exclusive designs for the same concern)?
2. For **orthogonal changes** (the common case): merge both. If side A adds a new method
   and side B refactors an existing one, the resolved file contains both the new method
   and the refactored code.
3. For **architectural conflicts** (rare): identify which design is more evolved, then
   carry forward the features and fixes from the other side into that design. The resolved design carries the behavioral contributions from both sides.
4. **Never resolve by choosing one side entirely.** Picking "ours" or "theirs" wholesale for a file is almost certainly wrong — the merge must integrate both sides' meaningful changes. The only exception is when one side made no meaningful changes to the file (e.g., a whitespace-only diff).
5. After resolving, verify the merged file is functionally complete:
   - Does it preserve all public API surface from both sides?
   - Does it preserve all behavioral features (not just code) from both sides?
   - Are tests from both sides still valid against the merged implementation?
Stage the resolved files and record the resolution through `instrukt repo
commit`. Submit the resulting candidate for fresh verification.

If main moved or its push is rejected, fetch and inspect the new ancestry.
Reconcile against current main, preserving all accepted deliveries. A new merge,
rebase or resolution produces a new candidate and requires verification through
`integrate`; never retry a changed commit directly onto main using the old green.
Do not rebase shared local main or overwrite another session's uncommitted files.

### Canonical checkout synchronization belongs to the runner

After promotion, confirm remote main. Each computer's next tick fetches and
fast-forwards its clean canonical checkout; the post-merge hook publishes docs
and hands the revision to blueprint. Do not perform a separate agent sync.
Dirty, misplaced or diverged checkouts are reported and preserved for their
owner; the runner retries after the obstruction is resolved.

### 5. Restore a red main first

If main becomes red despite the gate, revert the offending delivery first.
Compose and push the revert through `integrate` and obtain CI, or the local
`test` result when CI is absent. Prioritize that verified restoration before
subsequent deliveries build on the known-red baseline. Repair the cause in its
own todo. Judge whether the red-main evidence warrants a bug under
`sdlc/policy/todo-creation-discipline`; check existing work and file it once as
kind `bug` only when warranted, or add evidence to its existing owner.

If restoration cannot be completed, preserve the failure in this session.
Ask human-owned questions here, leaving the thread waiting for input. A
failure with no waiting thread is an anomaly; the runner's notification policy
may deliver only a reference to the session, never its failure content.

### 6. Complete the handoff and park

After work promotion, confirm the exact remote commit and record delivery evidence
with `instrukt todo deliver <slug> --commit <sha>`, supplying the confirmed delivered
commit under the work-record contract. instrukt itself removes the delivered
todo's clean `<slug>/work` worktree and deletes `<slug>/work` locally and on
`origin`, on each computer that holds them; issue no removal command for them
(`general/policy/shared-machine-resources`).

After delivery, inspect `instrukt todo show <slug>@<identity-prefix> --json` and
finish each recorded non-standing orchestrator thread of the delivered todo
through `runtime.finish_thread` under `sdlc/policy/todo-creation-discipline`
(never this standing integrator session). A worktree or branch whose removal is
still owed shows in that ledger and does not gate finishing. Preserve the
runtime thread record and retained attempt evidence.

When all current work is settled — no open candidate in verification, no
pending delivery or conflict, and no open human question — park as the standing
integrator and finish this thread under `general/procedure/event-handling/overview`.
The next `workflow.finalized`, `runner.ci_completed`, `runner.main_failing` or
`runner.dependency_failing` resumes this session in place; its mandate continues
across idle turns under the recorded standing-session binding.

## Outputs

- Exact verified candidates promoted and confirmed on remote main (with pending
  docs folded at delivery); ready one-off work integrated through the same gate.
- Durable delivery evidence, or findings and `--kickback` returned to the responsible todo.
- A failing main restored by a verified revert before separate repair work.

## Recovery

- On interruption, inspect the work record, integration worktree, submitted
  SHA and remote refs before continuing. Preserve an existing candidate and
  reconcile event replay; a restarted run is not another logical handoff.
- A failed push, fetch or observation remains an explicit unresolved operation.
- An unresolved conflict stays with the integrator and its governing contracts.
  Only missing human intent or a human-owned exposure is asked in the thread.
- A candidate already promoted needs delivery-record reconciliation, not
  another merge. Execute delivery (`instrukt todo deliver <slug> --commit <sha>`),
  then follow Step 6 directly: inspect `instrukt todo show <slug>@<identity-prefix> --json`
  and finish the delivered todo's recorded non-standing orchestrator
  thread(s) through `runtime.finish_thread`. Release an owed claim with
  `instrukt todo update <slug> --unclaim` and confirm the released claim. Record
  the handoff; apart from finishing completed todo orchestrator threads, the
  runtime owns session ending.
