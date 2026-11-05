---
description: 'Integrate exact verified candidates, publish settled direct commits through the same gate, and restore a red main before repair.'
visibility: 'public'
role: integrator
parameters:
  - name: slug
dispatch:
  agent: [codex, claude, gemini, deepseek]
  thinking_mode: slow
subscriptions:
  - workflow.finalized
  - runner.ci_completed
  - runner.main_ahead
  - runner.main_failing
---

# Integration — Procedure

## Required reads

- [general/principle/session-lifecycle](../../../../general/principle/session-lifecycle.md)
- [general/procedure/event-handling/overview](../../../../general/procedure/event-handling/overview.md)

## Goal

Merge finalized work and settled direct commits into a verified candidate,
advance canonical main to exactly that commit, and recover a red baseline
before repairing its cause. The standing integrator owns composition,
failure interpretation, promotion and recovery.

## Preconditions

- Main names the repository's canonical default branch; use its actual ref.
- `workflow.finalized` identifies a todo and finalized commit, or
  `runner.ci_completed` identifies the submitted candidate and result, or
  `runner.main_ahead` identifies local main ahead of remote main with no candidate pending, or
  `runner.main_failing` identifies remote main's head commit whose CI completed red.
- One standing integrator session is recorded per subscribing procedure and
  project; the runner starts or wakes that session.
- The integrator operates in the persistent integration worktree at
  `trees/_integration/`, separately from the project checkout and todo worktrees.
- The candidate's finalized evidence and branch are readable. Settled direct
  commits on local main are also available for candidate composition. Read each
  direct commit's git-note verdict: `approve` with its report binding or
  `exempt` with its author's reason makes it settled. Stop at the first commit
  with neither; that commit and its descendants stay outside the candidate.
- Project CI and required checks, or the configured `test` action when CI is
  absent, provide verification. GitHub branch protection guards main where CI exists.

## Steps

### 1. Route the activating event

- **`workflow.finalized`:** read the todo's artifacts, approval and matching
  finalized commit. Reconcile the handoff against repository state. A replay
  of the same project, todo and commit is the same logical handoff.
- **`runner.main_ahead`:** inspect local main and read the git-note verdicts
  on its direct commits. Compose settled direct commits through Step 2 even
  when no finalized todo is waiting.
- **`runner.ci_completed`:** reconcile the result with the exact submitted
  candidate and current GitHub check state. Proceed under Step 3 only when
  the required result is complete for that commit.
- **`runner.main_failing`:** take the recovery path under Step 5: revert the
  offending delivery first, then repair it in its own todo.

Event delivery retains idempotency keys and per-consumer cursors. A wake-up
carries a reason to inspect current evidence, not authority to repeat a merge
or promote whatever commit now occupies the integration branch.

### 2. Compose and publish the candidate

1. Fetch current remote main and candidate branches. Inspect the integration
   worktree before reuse; preserve any unique edits or an in-flight candidate.
2. Compose from accepted upstream main in the integration worktree. Merge the
   finalized branch, when present, and take settled direct commits on local main along.
   Read each direct commit's git-note verdict in ancestry order and take
   `approve` or `exempt`. Stop at the first commit with neither, retaining it
   and its descendants for settlement without replaying them under new SHAs.
   Uncommitted project-root files are outside the candidate and remain untouched.
3. Read the delivery context, diff and branch history. Where integration needs
   a commit, compose it around the delivery intent and record it through
   `instrukt repo commit` under `general/policy/commits`.
4. Run `instrukt docs fold <slug>` to reconcile the todo's pending documentation for delivery under the owning
   documentation contract before freezing the candidate. The work lane authors
   no new doc contract here. Include the resulting content in the commit verified.
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
the result `no_ci` for the exact candidate. Read
`instrukt config get actions.test` and run the project's configured command in
the integration worktree against that exact candidate. An unavailable or
failing action supplies no admission. This is the local gate;
builders, reviewers and the evaluator add no suite execution of their own.

- **Green:** fast-forward main to exactly the verified candidate and push it.
  Create no additional merge commit during promotion. Confirm the remote main
  points to that commit. GitHub's required checks guard that push.
- **Red:** read the failure. Return it to the responsible todo as findings,
  make a trivial fix within integration scope, or isolate the culprit among
  a batch. A change to intent or the prepared contract returns through the
  work orchestrator; a trivial integration correction is no substitute for it.
  Any changed candidate is pushed to `integrate` and verified again.

Direct commits with `approve` or `exempt` git-note verdicts receive the same verification as todo deliveries; they
supply no separate route to push main. A change to the candidate after a green
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
Reconcile against current main, preserving direct local commits. A new merge,
rebase or resolution produces a new candidate and requires verification through
`integrate`; never retry a changed commit directly onto main using the old green.
Do not rebase shared local main or overwrite another session's uncommitted files.

### REPO ROOT SYNC REQUIRED

After promotion, the integrator fast-forwards the project checkout's local main
to the exact promoted commit. The candidate took direct commits along, so local
main is an ancestor; confirm that ancestry again before syncing. Fetch remote
main and, at the project root on main, run
`git merge --ff-only --no-overwrite-ignore <promoted-commit>`. The checkout's hooks then publish and map the promoted content.

Inspect the working tree first. Leave unrelated dirty paths intact; if a dirty
or ignored path blocks the fast-forward, preserve it and route clearance to its
owner. Never stash, overwrite or discard another session's work, or rebase
shared main. If new direct commits have arrived, return them through their
git-note review and candidate verification gate before synchronizing; do not
force the checkout back to the promoted commit. A failed sync remains owed.

### 5. Restore a red main first

If main becomes red despite the gate, revert the offending delivery first.
Compose and push the revert through `integrate` and obtain CI, or the local
`test` result when CI is absent. Prioritize that verified restoration before
subsequent deliveries build on the known-red baseline. Repair the cause in its
own todo, filed as kind `bug` when the defect-evidence standard is met.

If restoration cannot be completed, preserve the failure in this session.
Ask human-owned questions here, leaving the thread waiting for input. A
failure with no waiting thread is an anomaly; the runner's notification policy
may deliver only a reference to the session, never its failure content.

### 6. Complete the handoff and park

After promotion, confirm the exact remote commit and record delivery evidence
with `instrukt todo deliver <slug> --commit <sha>`, supplying the confirmed delivered commit
under the work-record contract. Preserve working
resources until their delivery and cleanup obligations are established.

Re-run `instrukt todo work <slug>` after delivery: the flavor owes
`worktree.remove`, running `git worktree remove {worktree.path}` until
`worktree.exists` is absent. A failed removal remains owed; removal of the
worktree never removes the persistent integration worktree.

When all current work is settled, park as the standing integrator. The next
`workflow.finalized`, `runner.ci_completed`, `runner.main_ahead` or `runner.main_failing` resumes this session. Its mandate continues across idle turns under the recorded standing-session
binding.

## Outputs

- Exact verified candidates promoted and confirmed on remote main.
- Settled direct local-main commits included through the same gate.
- Durable delivery evidence, or findings returned to the responsible todo.
- A failing main restored by a verified revert before separate repair work.

## Recovery

- On interruption, inspect the work record, integration worktree, submitted
  SHA and remote refs before continuing. Preserve an existing candidate and
  reconcile event replay; a restarted run is not another logical handoff.
- A failed push, fetch or observation remains an explicit unresolved operation.
- An unresolved conflict stays with the integrator and its governing contracts.
  Only missing human intent or a human-owned exposure is asked in the thread.
- A candidate already promoted needs delivery-record reconciliation, not
  another merge. Execute delivery and cleanup through the shipped flavor's
  workflow bindings. Release an owed claim with `instrukt todo update <slug> --unclaim`
  and confirm the released claim. Record the handoff and remaining cleanup;
  the runtime owns session ending. A failed cleanup remains unresolved.
