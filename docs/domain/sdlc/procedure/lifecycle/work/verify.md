---
eval: true
description: 'Live behavioral verification phase. Ground candidate intent and fills, run Setup -> code-blind Driver -> anti-cheat Checker on isolated CI or local runtime slots, and return sanitized evidence-backed verification findings.'
visibility: public
role: verifier
parameters:
  - name: slug
    required: true
---

# Verify — Procedure

## Required reads

- [sdlc/policy/live-verification](../../../policy/live-verification.md)
- [sdlc/policy/preparation-artifact-quality](../../../policy/preparation-artifact-quality.md)
- [general/principle/parallel-work](../../../../../general/principle/parallel-work.md)
- [general/policy/shared-machine-resources](../../../../../general/policy/shared-machine-resources.md)

## Goal

Independently verify that the committed candidate HEAD works on its real user
or runtime surface without relying on builder claims, write sanitized,
publishable `verification-plan` and `verification-findings` artifacts to the
todo record, and record the verification disposition they support for that
exact HEAD.

## Preconditions

- The slug and clean candidate HEAD SHA were supplied by `todo-orchestrator`.
- **Placement:** `todo-orchestrator` reserved and bound this `verifier`
  attempt in `<slug>/work` after `build-reviewer` released, and you work there
  as a shared child at the candidate HEAD the attempt captured. Your session is
  distinct from the orchestrator and from every writer of the change, which is
  what admits your `pass`, `fail` or `blocked`
  (`general/principle/verification-independence`). Your disposition is
  admitted only over a clean tree at that captured HEAD: build output the
  repository ignores leaves it clean, and slots, raw evidence and every other
  run file live outside the worktree (Step 3). Recording the disposition
  releases the attempt (Step 6).
- Approved intent (`input`, plus `prd` for a feature or settled `diagnosis` for
  a bug) is readable by artifact name via
  `instrukt todo artifacts read {slug} <name>`.

## Steps

Every exit ends through Step 6, because your attempt holds `<slug>/work`
until its disposition is recorded. Wherever a step below returns
`Verdict: BLOCKED` or `Verdict: FAIL`, stop the run there, tear down what it
started (Step 6, item 1), write `verification-findings` carrying that verdict
and what was verified honestly (items 2 and 3), record
`--verification blocked` or `--verification fail` (item 4), and report it
(item 5).

### Project extension points

This procedure declares two project-level extension points
(`general/procedure/doc-snippet-authoring`):

- **`project/<project>/spec/verification-substrate`** — declares the project's
  system archetype(s), real observable surfaces, execution host, isolation
  primitive, concurrency cap, deterministic boot/seed entrypoints, sandbox
  credential-pool references, and declared blind spots.
- **`project/<project>/policy/reliability-checklist`** — holds the project's
  diff-triggered live verification checks born from confirmed real occurrences.

Resolve the repository's canonical default branch `<default-branch>` matching
repository placement: check tracking `HEAD` symrefs across `git remote` order
(`git symbolic-ref --quiet refs/remotes/<remote>/HEAD`, stripping
`refs/remotes/<remote>/`), falling back to `git config init.defaultBranch`, then
`main`. When `origin` exists, fetch `<default-branch>` from `origin` and set
`<canonical-ref>` to `origin/<default-branch>` (otherwise local
`<default-branch>`). Confirm `git rev-parse --verify "<canonical-ref>^{commit}"`
succeeds.

Handle every Git lookup fail-closed, keeping valid negative answers distinct
from fatal lookup errors:

- Resolve the merge-base as its own checked step (`base=$(git merge-base HEAD "<canonical-ref>")`)
  before running any `git diff "$base..HEAD"` command; never inline
  `$(git merge-base ...)` inside `git diff`. If `git rev-parse`, `git merge-base`,
  or `git log` exits non-zero, preserve its diagnostic and return
  `Verdict: BLOCKED` — never treating a failed lookup as an empty diff or absent
  fill.
- For every ancestry check (`git merge-base --is-ancestor <commit> <target>`),
  distinguish all three exit codes explicitly: exit `0` confirms ancestry; exit
  `1` confirms non-ancestry (routing to publication-lag handling or
  retained-stack synchronization); any other non-zero exit status (such as
  `128`) is a fatal lookup error that returns `Verdict: BLOCKED` with its
  diagnostic (never negating with `!`).

Resolve each fill across **both** `{slug}`'s candidate tree and published
canonical docs against freshly verified `<canonical-ref>`:

- **Candidate-owned additions, in-body edits, replacements, or retirements:**
  Check `instrukt docs index --target {slug}` and `{slug}`'s diff against the
  checked `$base` across both parent and sibling companion paths
  (`general/procedure/doc-snippet-authoring`):
  ```bash
  base=$(git merge-base HEAD "<canonical-ref>") && [ -n "$base" ] &&
  git diff "$base..HEAD" --name-only -- \
    'docs/project/spec/verification-substrate.md' \
    'docs/project/spec/verification-substrate.*.md' \
    'docs/project/policy/reliability-checklist.md' \
    'docs/project/policy/reliability-checklist.*.md'
  ```
  If the `&&` chain exits non-zero, preserve the diagnostic and return
  `Verdict: BLOCKED`. When a fill appears in `instrukt docs index --target {slug}`
  or matches that path filter, read `{slug}`'s candidate view (with
  `--target {slug}`). When `{slug}` retires the fill via
  an empty sibling companion (whose `--target {slug}` view renders
  an empty body) or removes it, treat that fill as absent for `{slug}` and
  report the absence explicitly in `verification-findings`.
- **Fills untouched by `{slug}` (published canonical docs):** Before trusting
  `instrukt docs index --project <project>` or a default (non-`--target`)
  snippet read to load a fill or judge it absent, confirm published project docs
  include `<canonical-ref>`'s latest commit touching that fill's path: read the
  `published commit` from `instrukt docs index --project <project>` and run
  `git log -1 --format=%H "<canonical-ref>" -- <path>` (verifying exit `0`
  before interpreting empty stdout as no history on `<canonical-ref>`). Whenever
  `<fill-commit>` is non-empty, run
  `git merge-base --is-ancestor "<fill-commit>" "<published-commit>"` when a
  published commit exists (exit `0` = current; exit `1` = confirmed publication
  lag; any other exit status = fatal lookup failure -> `Verdict: BLOCKED`). When
  the project has no published commit yet or exit `1` confirms detached
  post-merge publication lags behind `<canonical-ref>` (do not run a separate
  agent sync; canonical publication belongs to the runner and git hooks per
  `sdlc/procedure/lifecycle/integrate`), re-check briefly; if publication still
  lags behind `<canonical-ref>`, do not treat the fill as absent or verify
  against a stale fill — return `Verdict: BLOCKED` citing the published commit
  versus `<canonical-ref>`'s fill commit. Once publication is current for that
  path, read the delivered fill (without
  `--target`), or report it absent in `verification-findings` when unlisted.

### 1. Ground intent, diff, and candidate project fills

Read in full:

- Available artifact names via `instrukt todo artifacts list {slug}`, then read
  each present artifact body explicitly via
  `instrukt todo artifacts read {slug} <name>` (`input`, `log`, and `prd` /
  `diagnosis` / `implementation-plan` when present).
- Exact candidate HEAD and diff from the checked `$base` against the supplied
  candidate SHA (any non-zero exit returns `Verdict: BLOCKED`):
  ```bash
  git rev-parse HEAD &&
  base=$(git merge-base HEAD "<canonical-ref>") && [ -n "$base" ] &&
  git diff "$base..HEAD" --stat &&
  git diff "$base..HEAD"
  ```
- Both project extension-point fills resolved by the rule above (candidate view
  via `--target {slug}` when `{slug}` adds, edits, replaces, or retires the
  fill via parent or sibling companion; publication-checked reads
  from published canonical docs otherwise).

### 2. Plan dynamically (Layer 3) and classify tier

Apply the archetype matrix, execution-host rules, and dynamic tiering of
`sdlc/policy/live-verification`:

- **Tier 0 (no live run warranted):** write a concise Tier 0
  `verification-findings` citing the exact candidate HEAD SHA, reason, and
  `Verdict: TIER_0_SKIP`, record the `tier_0_skip` disposition and report it
  (Step 6, items 3 to 5), and stop.
- **Tier 1 / Tier 2:** compose the `verification-plan` you write in Step 6, naming:
  - exact candidate HEAD SHA and selected tier (`Tier 1` or `Tier 2`);
  - cheapest honest oracle(s), real surface(s), and execution host (`ci` or
    `local`) from Layer 1 or the archetype matrix;
  - user goals derived from approved intent plus targeted adversarial
    perturbations (cancel/abort mid-flow, reload/restart, duplicate submit,
    invalid boundary input, post-run persisted-state check);
  - every Layer 2 reliability trigger whose semantic condition matches the diff;
  - isolation slot strategy (single slot or CI job for Tier 1; parallel slots
    or CI matrix jobs bounded by Layer 1's concurrency cap for Tier 2).

### 3. Setup — provision isolated slot(s) on the declared execution host

First, for **every** execution host (`ci` and `local`), confirm the candidate
HEAD already includes any delivered `verification/` stack and seeder files
required by Layer 1 from `<canonical-ref>`: run
`git log -1 --format=%H "<canonical-ref>" -- verification/` (verifying exit `0`
before interpreting empty stdout as absent), and when `<stack-commit>` is
non-empty check `git merge-base --is-ancestor "<stack-commit>" HEAD` (exit `0` =
present; exit `1` = confirmed stale retained candidate -> return
`Verdict: BLOCKED` citing `<stack-commit>` so `todo-orchestrator` synchronizes
`<slug>/work` through `finalizer` Steps 1–4 followed by fresh independent
`build-reviewer` approval before re-running `verifier`; any other non-zero exit
status = fatal lookup failure -> return `Verdict: BLOCKED` with its diagnostic).

Select execution by archetype and Layer 1 `execution_host`:

- **CI-hosted container, browser, or mobile-simulator stack (default for Web,
  API, Mobile, and ERP archetypes):**
  - Confirm `todo-orchestrator` has already pushed the exact candidate HEAD SHA
    to `origin/<slug>/work`. If the candidate SHA is absent from the remote,
    return `Verdict: BLOCKED` naming the missing push.
  - **Pre-dispatch CI host and repository authority:** Resolve the project's
    authoritative code-host repository `<owner>/<repo>` matching repository
    configuration: read `instrukt config get github.repository` first; when
    unset, resolve the remote tracked by `<default-branch>`
    (`git config branch.<default-branch>.remote` and
    `git remote get-url <remote>`). Query the CI host explicitly against
    `repos/<owner>/<repo>` (never relying on an implicit CLI default repo; for
    GitHub, run
    `gh api "repos/<owner>/<repo>" --jq '[.full_name, .default_branch] | @tsv'`
    and
    `gh api "repos/<owner>/<repo>/git/ref/heads/<default-branch>" --jq '.object.sha'`),
    confirm `.full_name` equals `<owner>/<repo>` and `.default_branch` equals
    `<default-branch>`, and capture the authoritative default-branch head SHA
    (`<default-branch-sha>`). If `<owner>/<repo>` cannot be resolved, either CI
    host query fails, or the repository or default branch disagrees, refuse the
    trust path and return `Verdict: BLOCKED` citing the identity mismatch.
  - **Established default-branch verification workflow (standard path):**
    Trigger `workflow_dispatch` explicitly against `--repo "<owner>/<repo>"` on
    the CI-host-verified default branch `<default-branch>` (so
    GitHub executes the trusted default-branch workflow definition rather than
    any candidate-modified workflow file), passing the pushed candidate's
    `candidate_ref` / `candidate_sha` plus the read-only verification handoff
    inputs derived in Steps 1–2 (sanitized Layer 3 mission brief and
    adversarial perturbations, firing Layer 2 trigger list, and — when Checker
    runs in-job on a public repository — sanitized approved intent criteria and
    diff summary, using an authorized private-storage input reference when
    payloads exceed workflow input size limits so the fresh CI runner never
    depends on a local todo database or pending `--target` snippet reads). After completion
    and **before** trusting or accepting evidence (especially any
    public-repository in-job Checker verdict under
    `sdlc/policy/live-verification`), inspect the run explicitly on
    `repos/<owner>/<repo>` (for GitHub:
    `gh api "repos/<owner>/<repo>/actions/runs/<run_id>" --jq '[.repository.full_name, .path, .head_branch, .head_sha, .run_attempt, .html_url] | @tsv'`)
    and confirm **all** CI identities match: `.repository.full_name` equals
    `<owner>/<repo>`, `.path` matches the project's declared verification
    workflow path, `.head_branch` equals `<default-branch>`, `.head_sha` matches
    `<default-branch-sha>` (or a freshly re-queried current
    `git/ref/heads/<default-branch>` `.object.sha` on `<owner>/<repo>`),
    `.html_url`, `<run_id>`, and `.run_attempt` are recorded, and the
    checked-out subject SHA reported in the run's Setup/evidence output
    (`git rev-parse HEAD` of the checked-out subject) matches the exact
    candidate HEAD SHA (returning `Verdict: BLOCKED` on any missing or
    mismatched identity).
  - **Candidate modifies an established verification workflow:**
    - **In-place edit to an existing default-branch workflow path:**
      1. Execute **Run 1 (`trusted-dispatch`)**: trigger `workflow_dispatch`
         explicitly against `--repo "<owner>/<repo>"` on the CI-host-verified
         default branch `<default-branch>` (`inputs.candidate_sha`) and verify
         its post-run repository, workflow path, ref, revision SHA, and subject
         SHA as above to verify any non-workflow application or substrate
         changes independently under the trusted verifier.
      2. Execute **Run 2 (`candidate-workflow-test`)**: trigger
         `workflow_dispatch` against the pushed candidate branch ref
         (`origin/<slug>/work`, passing `candidate_ref`, `candidate_sha`, and
         sanitized Layer 3 test handoff inputs) so the candidate's modified
         dispatch trigger, input mapping, and shared entrypoint execute live at
         the exact candidate HEAD SHA. Run 2 supplies execution evidence only
         and never establishes its own verdict: the independent host Checker
         audits Run 2's raw CI evidence (delivered via private-repo CI
         artifacts or private authorized storage on public repos — returning
         `Verdict: BLOCKED` on a public repository lacking private authorized
         storage; a local probe never substitutes for required CI run evidence)
         alongside Run 1 and independent `build-reviewer` review of the
         workflow diff.
      3. Record **every** relied-on CI run (`Run 1` and `Run 2`) in the
         `CI Runs Relied On:` list below with its purpose, run URL, `run_id`,
         `run_attempt`, workflow ref/SHA, and executed subject SHA.
    - **Workflow rename or replacement at a new file path (no default-branch
      dispatch identity yet):** GitHub Actions requires a workflow file
      path to exist on the default branch before `workflow_dispatch` can target
      it; renaming or replacing an established verification workflow at a new
      path therefore lands as a **standalone workflow-migration change** with
      zero mixed application delta (if mixed with application changes, return
      `Verdict: BLOCKED` to split the workflow path migration standalone).
      Verify the standalone migration via the new path's branch-push or
      `pull_request` baseline smoke (`candidate-workflow-smoke`, confirming the
      shared verification entrypoint executes at exact candidate HEAD SHA with
      raw CI evidence audited by the independent host Checker over private-repo
      CI artifacts or private authorized storage — returning `Verdict: BLOCKED`
      if a public repository lacks private authorized storage), alongside
      independent static review of the new `workflow_dispatch` trigger and
      input wiring, explicitly noting that `workflow_dispatch` on the new file
      path cannot run until it lands on the default branch.
  - **First-time CI bootstrap (no verification workflow on the default branch
    yet):**
    - Permitted **only** for a standalone prerequisite bootstrap delivery
      carrying zero application-feature changes
      (`sdlc/policy/live-verification`: a standalone CI workflow bootstrap or
      path migration carries no application change). If a candidate mixes application changes with a missing
      default-branch workflow, return `Verdict: BLOCKED` naming the missing
      default-branch workflow so `todo-orchestrator` routes standalone
      prerequisite bootstrap first.
    - For a standalone first-time CI bootstrap candidate, its Layer 3 goal in
      Step 2 is the bootstrap contract itself (content-hashed image build/pull,
      isolated slot boot and seed, checked-in baseline end-to-end smoke on the
      real surface at exact candidate HEAD SHA, persisted-state check, and
      clean teardown — without requiring per-todo `workflow_dispatch` inputs or
      pre-existing Layer 2 triggers). Verify it by inspecting the branch-push
      or `pull_request` baseline smoke CI run (`first-bootstrap-smoke`,
      recording its run URL, `run_id`, `run_attempt`, bootstrap ref/SHA, and
      exact checked-out candidate HEAD SHA — never a synthetic merge commit)
      with the independent host Checker auditing raw CI run evidence over
      private-repo CI artifacts or private authorized storage on public repos
      (returning `Verdict: BLOCKED` if a public repository lacks private
      authorized raw-evidence storage; a local probe never substitutes for
      required CI run evidence) alongside independent `build-reviewer` review of
      the newly authored workflow file.
- **Local isolated execution (Docker-free CLI/daemon/temp-home archetypes, or
  explicit local-stack override):**
  - Confirm ≥20 GiB free disk space before building binaries or containers
    (`general/policy/shared-machine-resources`).
  - Hold each slot — its isolated roots, ports, data and the Driver's raw
    evidence location — in a temporary location outside the worktree that the
    project's own test harness or its language's managed temporary-directory
    facility creates and removes when the process holding it ends, following
    the slot recipe Layer 1 names. Ending that process is the slot's teardown,
    so the run never deletes a slot itself: a shell deletion trap stalls an
    unattended session on a destructive-command approval.
  - Boot the archetype's exact isolation primitive in that slot through
    Layer 1's boot entrypoint (a container stack under its declared project
    name and file on both boot and teardown; the candidate binary under
    isolated temp homes for CLI/daemon stacks, honoring Layer 1 and
    `sdlc/policy/testing` on inheriting the home and isolating application
    roots; or a native simulator or ephemeral tenant).
  - Run deterministic seed shortcuts to reach precondition state, and start
    every process the run spawns in a tracked process group that is reaped on
    both normal completion and error (`general/policy/shared-machine-resources`).

### 4. Driver — execute blind on the real surface

For standard Tier 1/2 runs, execute fresh black-box Driver session(s) (inside
the ephemeral CI runner on subscription-backed agent execution for CI-hosted
runs, or as fresh parallel subagents across isolated local slots per
`general/principle/parallel-work`) providing **only**:

- the booted surface address (URL, candidate binary path, API base URL,
  simulator ID, or sandbox channel) and seeded user role/credentials;
- the user-goal mission brief and adversarial perturbations from Step 2;
- instruction to interact exclusively through the real user/runtime surface,
  save raw evidence (recordings, screenshots, transcripts, observed state) onto
  an authorized non-public evidence surface (`sdlc/policy/live-verification`
  publication boundary — private-repo CI artifacts, private authorized storage,
  the ephemeral CI VM prior to in-job Checker audit on public repos, or the
  local slot's managed temporary location from Step 3), and report both goal
  pass/fail and an unprompted anomaly list of anything unexpected, slow, or
  broken.

Omit source code, repository file paths, git diffs, and implementation hints
from the Driver brief (`sdlc/policy/live-verification`).

For a **standalone first-time CI bootstrap** (`first-bootstrap-smoke`) or
**standalone workflow-path migration** (`candidate-workflow-smoke`) candidate
(Step 3), the branch-push or `pull_request` checked-in baseline smoke
exercising the shared verification entrypoint supplies the live execution on
the real surface (`sdlc/policy/live-verification`: a standalone CI workflow
bootstrap or path migration carries no application change) without requiring per-todo `workflow_dispatch`
inputs.

### 5. Checker — audit honesty, verify evidence, and triage anomalies

Inspect the Driver's (or standalone first-time bootstrap / workflow-path
migration baseline smoke's) raw evidence alongside the candidate diff, approved
intent, and any firing Layer 2 triggers:

- On **public repositories executing the trusted default-branch workflow**, the
  trusted workflow may execute Checker inside the ephemeral CI job before
  teardown and upload only the sanitized verdict/summary (or deliver raw
  evidence via private authorized storage).
- Whenever a candidate **authors (first-time bootstrap) or modifies (in-place
  edit or standalone path migration) the CI verification workflow itself**, the
  **independent host Checker** establishes the verdict from raw CI run evidence
  delivered over private-repo CI artifacts or private authorized storage (never
  trusting a candidate-run in-job summary; return `Verdict: BLOCKED` if a
  public repository lacks private authorized raw-evidence storage — a local
  probe credits only local behavior and never substitutes for required CI
  workflow evidence).

1. **Honesty check:** Confirm the real surface was exercised against the exact
   candidate HEAD SHA (and, on established non-workflow CI runs, under the
   trusted default-branch workflow) without JS-eval bypasses, internal-endpoint
   shortcuts, or mock downgrades. Re-run any compromised flow honestly before
   rendering a verdict.
2. **Contract check:** Confirm every required goal — the standalone
   bootstrap/migration baseline smoke contract (plus explicit note of the
   unexercised new-path `workflow_dispatch` trigger boundary and static
   workflow review) for `first-bootstrap-smoke` / `candidate-workflow-smoke`,
   or every approved intent goal, adversarial check, and firing Layer 2 trigger
   for standard Tier 1/2 runs — is proven by raw observable evidence (including
   persisted DB/file/ledger state where applicable).
3. **Anomaly triage:** Separate anomalies into:
   - **Blocking in-scope defects** (caused by or exposed on this candidate's
     change surface) — return `Verdict: FAIL` with actionable findings matching
     the `sdlc/procedure/lifecycle/fix-review-findings` contract below.
   - **Pre-existing unrelated observations** — record separately with sanitized
     reproduction steps for `deferrer` / `bug` intake, keeping this todo's
     scope bounded.

### 6. Tear down, record findings and the disposition

1. For local runs, end each slot as Layer 1's recipe names: stop every process
   group the run spawned (for a container stack, its declared teardown) and the
   process holding the slot's managed temporary location, which removes the
   slot and its raw evidence with it. Confirm zero orphaned processes remain
   and that `git status --porcelain` in `<slug>/work` prints nothing.
2. Enforce the publication boundary (`sdlc/policy/live-verification`,
   `sdlc/policy/security-awareness`): strip machine-local absolute paths, raw
   boot/environment dumps, unredacted payloads/headers, and sandbox credentials
   from every artifact you write and anything uploaded to a public CI
   artifact. Where a CI-hosted run retained raw evidence, the findings name the
   authorized home that holds it.
3. Write the publishable `verification-plan` (for Tier 1/2 runs) and
   `verification-findings` in the report format below with
   `instrukt todo artifacts write {slug} verification-plan '<content>'` and
   `instrukt todo artifacts write {slug} verification-findings '<content>'`.
4. From `<slug>/work` at the captured HEAD, record the disposition matching
   the report's `Verdict:` line:
   `instrukt todo stamp {slug} --verification pass|fail|blocked|tier_0_skip`.
   It binds that HEAD and the `verification-findings` version you just wrote,
   and releases your attempt; an attempt records one disposition. A refusal
   names what it lacks — a clean tree, the captured HEAD, a findings report —
   so restore that and record again, or report the refusal to
   `todo-orchestrator`.
5. Report the candidate SHA, the recorded disposition and any pre-existing
   observations to `todo-orchestrator`.

## Outputs

- Sanitized, publishable `verification-plan` artifact (for Tier 1/2 runs).
- Sanitized, publishable `verification-findings` artifact bound in its text to
  the exact candidate HEAD SHA with verdict (`PASS | FAIL | BLOCKED | TIER_0_SKIP`),
  missing extension-point fills (if any), fired Layer 2 triggers, authorized
  evidence references/summaries, Driver honesty confirmation, actionable
  blocking defect records (when `FAIL`), and pre-existing observations.
- A `--verification` disposition stamp bound to that exact HEAD and that
  `verification-findings` version, which released this `verifier` attempt.

### Report format

```text
VERIFICATION COMPLETE: {slug}

Candidate SHA: <exact-head-sha>
Tier: <Tier 0 | Tier 1 | Tier 2>
Execution Host: <ci | local>
CI Runs Relied On:
- <none (local run) | one entry per CI run relied on for this verdict:>
  - Purpose: <trusted-dispatch | candidate-workflow-test | candidate-workflow-smoke | first-bootstrap-smoke>
    Run / Attempt: <ci-run-url (run_id, run_attempt)>
    Workflow Ref / SHA: <workflow ref and SHA that executed>
    Executed Subject SHA: <checked-out candidate SHA tested>
    Evidence: <authorized private-repo/private-storage reference or sanitized summary>
Oracle: <surface / isolation primitive>
Missing Extension Fills: <none | list>
Local Evidence Summary: <sanitized local run summary or N/A>
Driver Honesty: <verified | re-run details>

Blocking Defects:
- None
  (or one structured entry per blocking defect compatible with fix-review-findings:)
  1. [<Requirement-Violation | Critical | Important>] <short defect title>
     Contract: <approved intent criterion or firing Layer 2 trigger>
     Method: <live surface, oracle, and exact reproducible flow executed (defect-evidence)>
     Evidence: <sanitized observed vs expected failure evidence at exact candidate SHA>
     Closure: <required behavioral outcome within frozen scope>

Pre-existing Observations:
- None (or sanitized reproduction steps for deferrer/bug intake)

Verdict: PASS | FAIL | BLOCKED | TIER_0_SKIP
```

## Recovery

- If a verification slot fails to boot due to a candidate defect (broken
  migration, crash on start, missing env wiring), tear down what the run
  started, then write `verification-findings` with the sanitized failure
  summary as a structured blocking defect (`Verdict: FAIL`) and record
  `--verification fail` (Step 6).
- If `<canonical-ref>` or its Git history lookup fails, the CI host's
  authoritative default branch cannot be verified or disagrees with
  `<default-branch>`, the candidate SHA is unpublished for a CI-hosted run, the
  candidate SHA predates delivered verification stack files on
  `<canonical-ref>`, detached post-merge publication lags behind
  `<canonical-ref>` on a required fill, a required substrate fill or workflow is
  missing, CI/host tooling is unavailable, or a required external credential is
  unprovisioned, tear down what the run started, write `verification-findings`
  naming the exact blocker and what was verified honestly with
  `Verdict: BLOCKED`, and record `--verification blocked` (Step 6) — never
  substituting mocked fallbacks or recording `pass` or `tier_0_skip`.
- If recording the disposition is refused, the refusal names what it lacks:
  restore a clean tree at the captured HEAD or the missing findings report and
  record again; when that is not yours to restore, report the refusal to
  `todo-orchestrator`, which resolves the attempt.

## See Also

- [sdlc/policy/live-verification](../../../policy/live-verification.md)
- [sdlc/procedure/verification-bootstrap](../../verification-bootstrap.md)
- [sdlc/procedure/lifecycle/work/review-build](review-build.md)
- [sdlc/procedure/lifecycle/overview](../overview.md)
