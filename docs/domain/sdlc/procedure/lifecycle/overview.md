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
- [sdlc/concept/defect-evidence](../../concept/defect-evidence.md)
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
     committed HEAD, and execute live verification for that exact HEAD per
     Step 4b (`sdlc/policy/live-verification`, pushing `origin/<slug>/work`
     first whenever the resolved execution host is `ci`).
  4. If static review returns `changes_requested` or `verifier` returns `FAIL`,
     dispatch admitted `builder`/`fixer`
     (`sdlc/procedure/lifecycle/fix-review-findings`, naming `review-findings`
     or `verification-findings`) to adjudicate and remediate findings, then
     re-run independent `build-reviewer` and live verification (Step 4b) on the
     resulting exact HEAD (or the unchanged exact HEAD with updated disposition
     evidence after an evidence-only static rejection pass). If `verifier`
     returns `BLOCKED`, resolve the missing substrate or prerequisite via Step
     4b before proceeding.
  5. Process any `deferrals` artifact via `deferrer`
     (`sdlc/procedure/lifecycle/work/deferrals`) and complete any Escape
     Ratchet obligation for real-occurrence `bug` todos (Step 5).
  6. Push the exact verified HEAD to `origin/<slug>/work` (if not already pushed
     for CI verification), stamp `instrukt todo stamp <slug> --finalize-ready`
     (directly as the claim-holding `todo-orchestrator`, or via `finalizer`
     when rebase/merge sync is needed — note that any new merge commit requires
     fresh exact-SHA independent static review and live verification
     reclassification per Step 4b), then release your claim with
     `instrukt todo update <slug> --unclaim` to hand off to `integrate`.

- **Non-trivial change, planned bug, or high blast radius:**
  1. Dispatch `planner` (`sdlc/procedure/lifecycle/prepare/plan-draft`) for
     blast-radius discovery, substrate readiness check, and
     `implementation-plan`, followed by independent `plan-reviewer`
     (`sdlc/procedure/lifecycle/prepare/review-plan`).
  2. Optionally dispatch `architect` + `architecture-reviewer` if structural
     boundaries or durable state change.
  3. Proceed to `builder` → independent `build-reviewer` and live verification
     (Step 4b, pushing `origin/<slug>/work` first when CI-hosted) →
     `fix-review-findings` remediation on `changes_requested` or `FAIL`, and
     Step 4b substrate/prerequisite resolution on `BLOCKED` → `deferrer` and
     Escape Ratchet (for real-occurrence `bug` todos) → exact-SHA push +
     `--finalize-ready` + `--unclaim` → `integrate`.

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
  5. Proceed to `builder` → independent `build-reviewer` and live verification
     (Step 4b, pushing `origin/<slug>/work` first when CI-hosted) →
     `fix-review-findings` remediation on `changes_requested` or `FAIL`, and
     Step 4b substrate/prerequisite resolution on `BLOCKED` → `deferrer` →
     exact-SHA push + `--finalize-ready` + `--unclaim` → `integrate`.

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
When a kickback to preparation leaves `<slug>/work` idle after build output has
already been created there (such as returning to a human approval gate after the
kickback), remove the regenerable build output created by this
todo's work sessions and their delegates (`general/policy/shared-machine-resources`),
leaving the clean committed tree, the todo branch and any published head intact,
and record any removal that cannot complete as an owed obligation. This does not
apply to a checkout still in active build or awaiting review before handoff.

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
2. **Check prerequisites, holds, published fill currency, and pre-planning
   substrate readiness.** If prerequisites are undelivered or a hold is active,
   preserve state and stop without guessing. Resolve the repository's canonical
   default branch `<default-branch>` and `<canonical-ref>`, and handle all Git
   lookups and ancestry exit codes (`0` = ancestor, `1` = confirmed
   non-ancestor, other non-zero = fatal lookup error), using the exact rules in
   `sdlc/procedure/lifecycle/work/verify` (tracking `HEAD` symrefs across
   `git remote` order, then `init.defaultBranch`, then `main`; fetch
   `<default-branch>` from `origin` when `origin` exists and confirm
   `git rev-parse --verify "<canonical-ref>^{commit}"`). On any fatal ref or
   history lookup error, never treat a fill or workflow as absent — resolve any
   stopped `planner` attempt FIRST on the clean, unheld todo, log the
   diagnostic, and stop/unclaim without dispatching. Before dispatching
   `planner` or fast-track `builder`, check `<canonical-ref>`:
   - **Published fill currency (for fills untouched by `{slug}`):** For
     `docs/project/spec/verification-substrate.md` and
     `docs/project/policy/reliability-checklist.md` (when not added, edited,
     replaced, or retired by `{slug}`'s own parent or sibling companion),
     compare the `published commit` reported by
     `instrukt docs index --project <project>` against
     `git log -1 --format=%H "<canonical-ref>" -- <path>` (checking exit `0`
     before interpreting empty stdout as absent; when `<fill-commit>` is
     non-empty, checking `git merge-base --is-ancestor "<fill-commit>" "<published-commit>"`
     with exit `0` = current, exit `1` = confirmed lag, other non-zero = fatal
     lookup error). When the project has no published commit yet or exit `1`
     confirms detached post-merge publication lags behind `<canonical-ref>` (do
     not run a separate agent sync; canonical publication belongs to the runner
     and git hooks per `sdlc/procedure/lifecycle/integrate`), re-check briefly;
     if publication still lags behind `<canonical-ref>`, resolve any
     early-stopped `planner` attempt FIRST on the clean, unheld todo
     (`instrukt todo attempt {slug} --role planner --attempt <n> --failed ...`,
     preserving legitimate committed preparation work), record the publication
     lag in `log`, and release the claim (`instrukt todo update {slug} --unclaim`,
     without `--hold`) so a subsequent runner tick retries once publication
     catches up (placing `--hold` only when canonical publication remains
     obstructed across retries).
   - **Retained-worktree fast-track synchronization (all execution hosts, `ci`
     and `local`):** Before dispatching fast-track `builder` on a retained
     `<slug>/work`, check `git log -1 --format=%H "<canonical-ref>" -- verification/`
     (exit `0` required) and, when `<stack-commit>` is non-empty,
     `git merge-base --is-ancestor "<stack-commit>" HEAD`. On confirmed
     non-ancestry (exit `1`), instruct `builder` in its brief to fetch and merge
     accepted `<canonical-ref>` into `<slug>/work` first before committing its
     bounded implementation and stamping `--build-complete` (on any exit status
     other than `0` or `1`, log the lookup failure and stop without
     dispatching).
   - **Default-branch CI verification workflow (for application deliveries whose
     surfaces resolve to `execution_host: ci` under
     `sdlc/policy/live-verification`):**
     - **Absent on `<canonical-ref>`:** Do not draft `implementation-plan` or
       admit `builder` against an unbootstrapped repository. If an in-flight
       `planner` stopped early on this condition, resolve its stopped attempt
       FIRST on the clean, unheld todo
       (`instrukt todo attempt {slug} --role planner --attempt <n> --failed ...`,
       preserving any legitimate committed preparation work). Then reuse an
       already-linked prerequisite bootstrap todo or establish and unfreeze a
       new standalone prerequisite bootstrap todo `<bootstrap-slug>` (carrying
       zero mixed application changes, with pre-entry reviewed pending Layer 1
       and Layer 2 fills pushed to `origin/<bootstrap-slug>/work` and no local
       `refs/heads/<bootstrap-slug>/work` left behind) following the tracked
       pre-entry sequence in `sdlc/procedure/verification-bootstrap` Step 4.
       Read `{slug}`'s current prerequisites from `instrukt todo show {slug}`,
       append `<bootstrap-slug>` without dropping existing edges
       (`instrukt todo update {slug} --after <existing-prereqs>,<bootstrap-slug>`,
       or `--after <bootstrap-slug>` when none exist), and release the claim
       (`instrukt todo update {slug} --unclaim`) **without** setting `--hold` so
       `{slug}` automatically becomes eligible once `<bootstrap-slug>` delivers
       onto canonical main (if woken earlier while `<bootstrap-slug>` is still
       undelivered on `<canonical-ref>`, simply unclaim and stop).
     - **Present on `<canonical-ref>` (and published fills current):** Proceed
       directly to normal preparation (`plan-draft` → `plan-reviewer`) or
       fast-track `builder`. Trusted default-branch CI verification executes
       from the CI-host-verified `<default-branch>`, and readers resolve Layer 1
       / Layer 2 fills per `sdlc/procedure/lifecycle/work/verify` (candidate
       `--target {slug}` for `{slug}`-owned parent or sibling-companion
       additions, edits, replacements, or retirements; publication-checked
       `instrukt docs get <id>` from published canonical docs for untouched
       fills) without ad hoc merges during preparation.
<!-- planned-change:reuse-retained-worker-sessions -->
3. **Select and dispatch the next primitive.** Every admitted role that shares
   `<slug>/work` (`intaker`, `architect`, `planner`, `diagnoser`,
   `drift-analyzer`, `builder`, `fixer`, `build-reviewer`, `prd-reviewer`,
   `architecture-reviewer`, `plan-reviewer`, `deferrer`, `finalizer`) uses
   `runtime.dispatch` above: reserve each attempt via
   `instrukt todo attempt {slug} --role <role> --procedure <id> --args {slug} --brief -`,
   delegate with shared isolation in `<slug>/work`, and immediately bind via
   `instrukt todo bind`. Only one workspace attempt may be active at a time;
   completion stamps release it atomically. Live verification (`verifier`) is
   not a shared `<slug>/work` `runtime.dispatch` child; it runs per Step 4b in
   a separate one-off worktree session (`runtime.start_thread` /
   `runtime.inspect_thread`) so the shared-workspace admission guarantee
   remains unbroken.
<!-- change:reuse-retained-worker-sessions -->
3. **Select and dispatch the next primitive.** Every admitted role that shares
   `<slug>/work` (`intaker`, `architect`, `planner`, `diagnoser`,
   `drift-analyzer`, `builder`, `fixer`, `build-reviewer`, `prd-reviewer`,
   `architecture-reviewer`, `plan-reviewer`, `deferrer`, `finalizer`) reserves
   each round via
   `instrukt todo attempt {slug} --role <role> --procedure <id> --args {slug} --brief -`,
   then continues its retained worker or starts a fresh one as the
   continue-or-start-fresh rule of the orchestration loop in
   `general/procedure/orchestration` decides:
   - **Continue** — bind the new attempt to the role's prior session via
     `instrukt todo bind`, then send its brief through `runtime.message_child`
     above. The producers whose follow-up rounds continue are `intaker`,
     `architect`, `planner`, `builder`, `fixer`, `deferrer` and `finalizer`;
     the reviewers whose finding-closure re-checks continue are `prd-reviewer`,
     `architecture-reviewer`, `plan-reviewer` and `build-reviewer`.
   - **Start fresh** — delegate with shared isolation in `<slug>/work` through
     `runtime.dispatch` above and immediately bind via `instrukt todo bind`.

   Only one workspace attempt may be active at a time; completion stamps
   release it atomically. Live verification (`verifier`) is not a shared
   `<slug>/work` `runtime.dispatch` child; it runs per Step 4b in a separate
   one-off worktree session (`runtime.start_thread` /
   `runtime.inspect_thread`) so the shared-workspace admission guarantee
   remains unbroken.
<!-- /planned-change:reuse-retained-worker-sessions -->
4. **Verify review substance and contest invalid findings before acting on verdicts.**
   Confirm the findings artifact exists, covers all applicable review lanes,
   discloses each finding's `Method:` (finding format in
   `sdlc/procedure/lifecycle/review`), and justifies clean approvals
   specifically for the target. Maintain a bidirectional adversarial stance:
   distrust unverified claims and audit every finding's `Method:` and premises
   against its governing contract (`sdlc/concept/defect-evidence` for defect
   claims) and the frozen intent rather than forwarding reviewer demands as
   mandatory closures. Route review remediation through
   `sdlc/procedure/lifecycle/fix-review-findings` (contest-first) so valid
   findings are fixed, invalid or out-of-bounds findings are rejected on the
   record by the producer, and repeated failures on the same mechanism or
   invariant trigger re-derivation — cutting back to a simpler policy-compliant
   boundary whenever it satisfies every approved criterion and eliminates the
   failing interaction instead of layering defensive machinery
   (prune-before-patch in `sdlc/procedure/lifecycle/review`).
   - **4b. Live verification disposition for the exact candidate HEAD.** Load
     `sdlc/policy/live-verification` (`instrukt docs get sdlc/policy/live-verification`)
     and classify every clean candidate HEAD (including any replacement HEAD
     produced by `fixer` or `finalizer`):
     - **Tier 0:** `todo-orchestrator` records the concise Tier 0 disposition
       naming the exact candidate HEAD SHA and reason via
       `instrukt todo artifacts write {slug} verification-findings '<content>'`
       and skips `verifier` dispatch.
     - **Emergency hotfix deferral (`HOTFIX_DEFERRED`):** Permitted solely
       under `sdlc/policy/live-verification` Exceptions while preserving every
       existing `sdlc/policy/definition-of-done` control; `todo-orchestrator`
       records `Verdict: HOTFIX_DEFERRED` in `verification-findings` citing the
       exact candidate HEAD SHA and the tracked follow-up todo slug/URL that
       delivers post-hotfix live verification (and `escape-assessment` when the
       hotfix is a real-occurrence bug).
     - **Tier 1 / Tier 2:**
       - Whenever the resolved execution host (from Layer 1
         `verification-substrate`, or when absent from the archetype matrix in
         `sdlc/policy/live-verification`) is `ci`, push the clean candidate
         HEAD to `origin/<slug>/work` (`git push origin HEAD:<slug>/work`)
         before dispatching `verifier` so the CI runner can fetch and verify
         that exact commit SHA.
       - Start an independent `verifier`
         (`sdlc/procedure/lifecycle/work/verify`) distinct from the
         orchestrator and writers on this host as a separate one-off worktree
         session branched from the exact candidate HEAD SHA
         (`runtime.start_thread` — not a `runtime.dispatch` shared child of
         `<slug>/work`), and read its completed output via
         `runtime.inspect_thread`. Persist the verifier's returned sanitized
         `verification-plan` and `verification-findings` payloads verbatim via
         `instrukt todo artifacts write` (noting the verifier's session ID and
         exact candidate HEAD SHA). When `verification-findings` includes
         pre-existing observations, append any not already recorded as new
         observation entries (`sdlc/procedure/lifecycle/work/deferrals`) to
         `{slug}`'s `deferrals` artifact via
         `instrukt todo artifacts write {slug} deferrals '<content>'` —
         preserving all existing `deferrals` entries — so `deferrer` triages
         every build and live-verification observation together before
         finalization.
       - On `FAIL`, dispatch an admitted workspace writer (`builder`/`fixer`
         via `instrukt todo attempt` / `bind`) through
         `sdlc/procedure/lifecycle/fix-review-findings` naming
         `verification-findings`, then re-run static `build-reviewer` and live
         verification (Step 4b) on the resulting exact HEAD.
       - On `BLOCKED`, resolve the named blocker:
         - If the candidate SHA was not yet published to `origin/<slug>/work`
           for CI verification, push the clean candidate HEAD to
           `origin/<slug>/work` and re-run `verifier`.
         - If detached post-merge publication lagged behind `<canonical-ref>` on
           a required fill, re-check `instrukt docs index --project <project>`
           briefly and re-run `verifier` once publication includes
           `<canonical-ref>`'s fill commit; if publication still lags behind
           `<canonical-ref>`, record the lag in `log` and release the claim
           (`instrukt todo update {slug} --unclaim`, without `--hold`) so a
           subsequent runner tick retries once publication catches up (placing
           `--hold` only when canonical publication remains obstructed across
           retries).
         - If a retained `<slug>/work` candidate SHA (for either `ci` or `local`
           hosts) is confirmed by `git merge-base --is-ancestor "<stack-commit>" HEAD`
           (exit `1`, never conflated with fatal lookup errors) to predate
           delivered `verification/` stack or seeder files on `<canonical-ref>`
           — including when resuming after a standalone prerequisite bootstrap
           or substrate-extension todo delivers onto `<canonical-ref>` —
           process any unprocessed `deferrals` artifact first via `deferrer`
           (`sdlc/procedure/lifecycle/work/deferrals`, satisfying `finalize`'s
           deferral precondition), then synchronize through the post-review
           synchronization owner, `finalizer` (`instrukt todo attempt` / `bind`
           executing `sdlc/procedure/lifecycle/work/finalize` Steps 1–4: merge
           accepted `<canonical-ref>`, push `origin/<slug>/work`, and release
           via `--role-complete` without stamping `--finalize-ready`), then
           obtain fresh independent exact-SHA `build-reviewer` `APPROVE` and
           re-run live verification (Step 4b) against that exact merged HEAD
           before recording `--finalize-ready`.
         - If missing **first-time CI verification workflow** (no default-branch
           verification workflow exists yet) blocks an application todo, **always**
           route it as a **standalone prerequisite bootstrap todo** (never
           mixing application changes with first-time CI workflow creation, per
           `sdlc/policy/live-verification`, Standalone CI workflow bootstrap or
           path-migration rule): establish and unfreeze `<bootstrap-slug>` with
           pre-entry reviewed pending Layer 1 and Layer 2 fills pushed to
           `origin/<bootstrap-slug>/work` (leaving no local
           `refs/heads/<bootstrap-slug>/work`) following
           `sdlc/procedure/verification-bootstrap` Step 4, append
           `<bootstrap-slug>` to `{slug}`'s prerequisites without dropping
           existing edges
           (`instrukt todo update {slug} --after <existing-prereqs>,<bootstrap-slug>`,
           or `--after <bootstrap-slug>` when none exist), and release the
           claim (`instrukt todo update {slug} --unclaim`, without `--hold` so
           delivery of `<bootstrap-slug>` automatically re-enables this todo);
           once `<bootstrap-slug>` delivers onto canonical main, synchronize the
           retained `<slug>/work` through the deferrals-checked `finalizer`
           Steps 1–4 -> fresh `build-reviewer` -> `verifier` path above so the
           verified exact candidate SHA includes the delivered verification
           stack and seeders.
         - If an **existing** substrate needs an in-scope update (on a repo
           whose default-branch verification workflow already exists, or on a
           `local` repo):
           - For a code-only stack or seeder fix within the already-approved
             plan and Layer 1 contract (no Layer 1 / Layer 2 documentation
             delta), reserve and bind `builder` (`--role builder --procedure sdlc/procedure/lifecycle/work/build`)
             to update the substrate (`sdlc/procedure/verification-bootstrap`
             Steps 2–3 when applicable) in `<slug>/work`, commit, and release
             via `--build-complete`, then run static `build-reviewer` and live
             verification (Step 4b).
           - For any Layer 1 / Layer 2 pending contract update or plan revision
             within approved intent, route through the owning preparation path:
             reserve and bind `planner` (`--role planner --procedure sdlc/procedure/lifecycle/prepare/plan-draft`)
             to update pending fills (Step 4b) and `implementation-plan`
             (`--role-complete`), obtain independent `plan-reviewer` approval
             (`sdlc/procedure/lifecycle/prepare/review-plan`), and only after
             plan approval dispatch `builder` (`work/build`, `--build-complete`)
             for any implementation delta, followed by static `build-reviewer`
             and live verification (Step 4b).
           - Exceeding this todo's frozen scope, file a tracked prerequisite
             substrate todo, preserve existing prerequisites with `--after`,
             and unclaim (`--unclaim`, without `--hold`) as above.
         - If blocked by external host tooling, missing private raw-evidence
           storage for a public-repo workflow edit/bootstrap, or a brand-new
           external paid/legal third-party credential, place an explicit hold
           (`instrukt todo update {slug} --hold --reason "..."`) naming the
           prerequisite or credential-pool request.
5. **Finalize and hand off to `integrate`.** Confirm the worktree is clean,
   no workspace attempt is active, `origin/<slug>/work` matches the exact
   independently approved HEAD commit SHA, current exact-SHA
   `verification-findings` holds (`PASS`, `TIER_0_SKIP`, or `HOTFIX_DEFERRED`
   with tracked follow-up todo under `sdlc/policy/live-verification` Exceptions,
   and zero unresolved `FAIL` or `BLOCKED`), and all required approvals hold.
   For every `kind: bug` todo originating from a real occurrence
   (`sdlc/concept/defect-evidence`) on any route — except an emergency hotfix
   carrying `HOTFIX_DEFERRED`, whose tracked follow-up todo owns post-hotfix
   `escape-assessment` — confirm `escape-assessment`
   (`sdlc/policy/live-verification`) is persisted via
   `instrukt todo artifacts write {slug} escape-assessment '<content>'` and
   that `regression-graduated` tests ship on the reviewed candidate HEAD while
   any `trigger-added` or `substrate-upgraded` work outside minimal bug-fix
   scope (`sdlc/policy/definition-of-done-lanes`, Bug-fix review) is filed as a
   tracked follow-up cited in `escape-assessment`. Record
   `instrukt todo stamp {slug} --finalize-ready` (which atomically emits
   `workflow.finalized`). Then remove the regenerable build output created in
   `<slug>/work` by this todo's work
   sessions and their delegates (`general/policy/shared-machine-resources`),
   leaving the clean committed tree, the todo branch and the published head
   intact as bound by the readiness record; if removal cannot complete, record
   it as an owed obligation without reopening the handoff. Then release the
   claim with `instrukt todo update {slug} --unclaim`. Keep this orchestrator
   thread wakeable (never retire it before delivery — `integrate` retires it
   once the todo is delivered).
6. **Retire on direct withdrawal.** When this orchestrator withdraws or removes
   a todo directly (for example under triage, retraction, or a no-fix exit),
   finish all record writes (`log`, `instrukt todo remove {slug}`) and confirm
   the removal outcome first, preserve any unfinished human or worker
   obligations, and then retire this terminal orchestrator thread via the
   configured `runtime.retire_thread` action (converging idempotently on replay
   and never retiring standing agents).

## Outputs

- Verified candidate commit pushed to `origin/<slug>/work` with an independent
  exact-SHA `build_review` `approve` stamp, current exact-SHA
  `verification-findings` (`PASS`, `TIER_0_SKIP`, or `HOTFIX_DEFERRED`),
  `escape-assessment` for real-occurrence bugs, `--finalize-ready` handoff to
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
  approval and live verification disposition (Step 4b), push, and re-stamp
  `--finalize-ready`.

## See also

- [sdlc/procedure/lifecycle/integrate](integrate.md)
- [sdlc/procedure/lifecycle/work/build](work/build.md)
- [sdlc/procedure/lifecycle/work/review-build](work/review-build.md)
- [sdlc/procedure/lifecycle/work/verify](work/verify.md)
- [sdlc/procedure/lifecycle/work/finalize](work/finalize.md)
