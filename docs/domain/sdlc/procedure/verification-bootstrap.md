---
eval: true
description: Autonomously bootstrap a project's live verification substrate (archetype classification, portable CI-first verification stack and seeders where earned, live self-healing probe, and Layer 1/Layer 2 project fills).
visibility: public
---

# Verification Bootstrap — Procedure

## Required reads

- [sdlc/policy/live-verification](../policy/live-verification.md)
- [general/procedure/doc-snippet-authoring](../../../general/procedure/doc-snippet-authoring.md)
- [general/policy/shared-machine-resources](../../../general/policy/shared-machine-resources.md)

## Goal

Autonomously establish a project's repeatable, isolated live verification
substrate and initial Layer 1 / Layer 2 project fills without manual first-run
prototyping.

## Preconditions

- The target repository is available in either:
  - a clean adoption or greenfield workspace (`sdlc/procedure/project-analysis`
    or `sdlc/procedure/greenfield-intake`), where one session executes Steps 1–5
    directly; or
  - a tracked todo lifecycle following the pre-entry and build split defined in
    Step 4 below (pre-entry icebox authoring and independent review of Steps 1
    and 4 before active entry under `sdlc/policy/todo-creation-discipline`,
    followed by `planner` / `plan-reviewer` and admitted `builder` executing
    Steps 2–3 in `<slug>/work`).
- Repository manifests, runtime entrypoints, and existing `docs/project/`
  snippets are readable.

## Steps

### 1. Classify archetype, real surfaces, and execution host

Inspect repository manifests, runtime entrypoints, migrations, environment
templates, and existing `docs/project/` specs. Map every real user or runtime
surface to its cheapest honest oracle and execution host using
`sdlc/policy/live-verification`:

- Containerized web/API/worker stacks and mobile simulators default to an
  **ephemeral CI runner** (`execution_host: ci`, avoiding Docker-in-Docker
  inside company containers and keeping compute portable across hosted and
  self-hosted runners).
- Docker-free archetypes (CLI/daemon temp-home smokes, libraries and
  pipelines) use native lightweight oracles without scaffolding container or
  browser machinery.
- A repository that holds prose only has no real surface to map: record that
  in Layer 1 and end the bootstrap there.

### 2. Author the verification stack by archetype (when earned)

- **Containerized Web / API / worker archetypes:**
  1. Create `verification/Dockerfile.verify` installing only the dependencies
     required to drive and observe the real surfaces (for example, Chromium and
     WebKit browser drivers for web apps, DB/cache clients, and seed helpers).
  2. Create `verification/docker-compose.verify.yml` parameterized for parallel
     isolated slots (`docker compose -p verify-<slot> -f verification/docker-compose.verify.yml`):
     ephemeral unnamed/tmpfs volumes, dynamic host port allocation, local
     protocol-faithful emulators for external services (DB, Redis, MinIO,
     Mailpit, local OIDC, webhook emulators), and image tag parameterized by
     the content hash of `verification/Dockerfile.verify` plus dependency
     lockfiles (replacing any floating `:latest` tag).
  3. Create deterministic state-seeding helpers under `verification/seed/` for
     Setup-stage account, tenant, and fixture initialization.
  4. Wire CI image publication and reusable live verification execution
     (composing shared workflows per `sdlc/policy/ci-pipeline-conventions` with
     parameterized runner labels so hosted and self-hosted runners swap
     cleanly) to build/push `<registry>/<repo>-verify:<sha256-prefix>` on
     Dockerfile/lockfile changes and run Setup -> blind Driver -> Checker
     on ephemeral CI VMs. Configure the verification workflow as a thin event
     trigger delegating Setup -> Driver -> evidence logic to one shared
     reusable entrypoint (`sdlc/policy/no-inline-logic`) invoked by both
     trusted default-branch `workflow_dispatch` (accepting `candidate_ref`,
     `candidate_sha`, and read-only Layer 3 handoff inputs and explicitly
     checking out `inputs.candidate_sha` as the subject) and branch
     `push`/`pull_request` runs (passing default baseline-smoke inputs against
     the exact candidate head SHA so workflow edits exercise the shared path).
     Enforce the publication boundary of
     `sdlc/policy/live-verification`: on private repositories, raw evidence may
     upload to repository CI workflow artifacts with retention TTL; on public
     repositories, an in-job Checker uploading only sanitized, publication-safe
     verdicts/summaries is valid only for trusted default-branch runs, whereas
     first-time bootstrap and workflow-modifying candidates deliver raw CI run
     evidence exclusively to private authorized storage for independent host
     Checker audit (returning `BLOCKED` if private authorized storage is
     unavailable; a local probe never substitutes for required CI run
     evidence).
- **Mobile app archetypes:** configure the native iOS Simulator / Android
  Emulator boot, deterministic app-state seeding, and reusable CI runner
  workflow (`macos-latest` / emulator runner, portable to self-hosted hosts,
  honoring the same public/private artifact boundary) without forcing Docker
  Compose around native simulators.
- **ERP / business-ops tenant archetypes:** configure ephemeral/staging tenant
  provisioning and deterministic seed helpers under `verification/seed/`
  (adding Compose definitions only when the ERP stack itself runs locally as
  containers).
- **CLI / daemon / libraries archetypes:** omit container stacks and document
  the exact candidate-binary, isolated-root or corpus entrypoints directly in
  Layer 1.

### 3. Resolve dependencies, probe live, and self-heal

1. Resolve external dependencies in strict priority order:
   - local protocol-faithful emulators first;
   - existing org-level sandbox credentials resolved from `instrukt secrets` or
     the repository's CI secret store (verifying configured secret names for
     the probe without printing values);
   - only when a brand-new paid or legal third-party account lacks both, record
     it as a declared blind spot and continue probing all other surfaces.
2. Confirm ≥20 GiB free disk space before any local build or container probe
   (`general/policy/shared-machine-resources`).
3. Boot one isolated slot using the authored execution path:
   - When probing a **newly authored CI workflow not yet on the default branch**,
     trigger it through a branch-push or `pull_request` workflow run on the
     pushed bootstrap branch executing the repository's **checked-in
     self-contained baseline smoke** (explicitly checking out the exact
     candidate head SHA and proving image build/pull, boot, seed, one real
     surface flow, and teardown without requiring `workflow_dispatch` inputs)
     or run a local probe when host tooling permits. When this bootstrap runs
     as a tracked todo, that exact-SHA baseline smoke satisfies the first-time
     CI bootstrap admission path in `sdlc/procedure/lifecycle/work/verify` so
     the bootstrap todo can clear finalization and land the workflow on the
     default branch alongside independent `build-reviewer` approval.
   - Once the verification workflow lands on the default branch,
     `workflow_dispatch` runs against the trusted default-branch ref with
     explicit `candidate_ref` / `candidate_sha` and per-todo Layer 3 read-only
     handoff inputs (`sdlc/procedure/lifecycle/work/verify`).
4. Exercise one real end-to-end flow on every declared surface and verify
   persisted state afterwards.
5. Absorb every missing package, browser binary, font, or boot step discovered
   during the probe directly into `verification/Dockerfile.verify`,
   `verification/docker-compose.verify.yml`, or `verification/seed/`, then re-run
   from clean state until one command or CI run boots, seeds, and passes
   cleanly.
6. Tear down any local probe slot completely (for Compose:
   `docker compose -p verify-<slot> -f verification/docker-compose.verify.yml down -v`;
   for native/temp-home: kill process groups and remove temp dirs) and confirm
   zero orphans remain.

### 4. Author Layer 1 and Layer 2 project fills

Following `general/procedure/doc-snippet-authoring`,
`general/policy/documentation-discipline`, and the extension points declared in
`sdlc/procedure/lifecycle/work/verify`:

1. Author `docs/project/spec/verification-substrate.md`
   (`project/<project>/spec/verification-substrate`) documenting:
   - System archetype(s), real observable surfaces, and `execution_host` (`ci`
     or `local`);
   - Isolation primitive, exact invocation/teardown commands, and max parallel
     concurrency;
   - Content-hashed image build/pull recipe and deterministic seed entrypoints
     (when applicable);
   - Org-level sandbox credential-pool references (secret names only, omitting
     secret values);
   - Explicitly declared blind spots (any non-relocatable runtime home or
     external paid/legal SaaS lacking both a local emulator and a pool
     credential).
2. Author `docs/project/policy/reliability-checklist.md`
   (`project/<project>/policy/reliability-checklist`) establishing the project's
   Layer 2 diff-triggered incident catalog — initially empty, or seeded solely
   with confirmed historical real occurrences carrying inspectable source
   addresses (`sdlc/concept/defect-evidence`).
3. Apply the authoring and `docs/project/index.yaml` lifecycle matching the
   entry route:
   - **Direct adoption or greenfield scaffolding (`sdlc/procedure/project-analysis`
     or `sdlc/procedure/greenfield-intake`):** Write live fills directly, add
     both IDs under `listed` in `docs/project/index.yaml`, run
     `instrukt docs render`, and verify with `instrukt docs validate` and
     `instrukt docs render --check`.
   - **Tracked repository todo lifecycle:** Before active lifecycle entry
     (`sdlc/policy/todo-creation-discipline`), the creating/filing session:
     1. Creates `<slug>` in the icebox (`instrukt todo create <slug> --kind change --icebox ...`,
        carrying zero mixed application changes).
     2. In a short-lived temporary worktree branched from `<canonical-ref>` (on
        a temporary local branch or detached HEAD, never leaving a local
        `refs/heads/<slug>/work` branch), authors both fills as pending
        documentation (`delivered_by: [<slug>]` for new fills, omitted from
        `docs/project/index.yaml` while pending per
        `general/spec/snippet-authoring-schema` and
        `general/procedure/doc-snippet-authoring`; pending markers or sibling
        companions for edits to already-delivered fills), verifies with
<!-- planned-change:author-recorded-review-routing -->
        `instrukt docs validate` and `instrukt docs render --check`, commits via
        `instrukt repo commit --todo <slug>`, and pushes the exact commit to
<!-- change:author-recorded-review-routing -->
        `instrukt docs validate` and `instrukt docs render --check`, commits via
        `instrukt repo commit --todo <slug> --review`, and pushes the exact commit to
<!-- /planned-change:author-recorded-review-routing -->
        `origin/<slug>/work` (`git push origin HEAD:refs/heads/<slug>/work`).
     3. Obtains independent commit-review `APPROVE`
        (`sdlc/procedure/lifecycle/work/review-commits`) bound to that exact
        commit SHA (resolving any required changes and re-pushing
        `origin/<slug>/work`).
     4. Removes the temporary worktree and deletes any temporary local branch
        (confirming no local `refs/heads/<slug>/work` remains so native
        placement creates `<slug>/work` cleanly from `origin/<slug>/work`), then
        enters `<slug>` into the active lifecycle via
        `instrukt todo unfreeze <slug>`.

     In the active lifecycle, `planner` (`sdlc/procedure/lifecycle/prepare/plan-draft`) plans only Steps 2–3
     against those reviewed fills without assigning documentation tasks to
     `builder` (`sdlc/procedure/lifecycle/prepare/plan-draft`), and `plan-reviewer` approves
     `implementation-plan` before build. During build, `builder` executes only
     Steps 2–3 (stack authoring and live probe); if the live probe discovers an
     irreducible contract delta to Layer 1 or Layer 2 (such as an uncovered
     declared blind spot or changed execution host / isolation primitive),
     `builder` stops and escalates to `todo-orchestrator` so the owning
     preparation route (`planner` → `plan-reviewer`) updates and
     re-reviews the pending fills and plan. At delivery
     (`sdlc/procedure/lifecycle/integrate`), `integrate` runs
     `instrukt docs fold <slug>`, adds any newly delivered fill IDs under
     `listed` in `docs/project/index.yaml`, runs `instrukt docs render`, and
     verifies with `instrukt docs validate` and `instrukt docs render --check`
     in the exact folded candidate verified on `origin/integrate`.

### 5. Surface only irreducible external credential gaps

If a declared surface requires a brand-new external paid or legal third-party
account absent from the org sandbox credential pool and unemulatable locally,
record it under declared blind spots in `verification-substrate.md` (via
pre-entry authoring or preparation when inside a tracked todo) and report that
single credential-pool request once while completing the rest of the bootstrap.

## Outputs

- `docs/project/spec/verification-substrate.md` (Layer 1) and
  `docs/project/policy/reliability-checklist.md` (Layer 2) — live, listed under
  `listed` in `docs/project/index.yaml`, and rendered into `AGENTS.md` on direct
  adoption/greenfield entry; or pre-entry independently reviewed pending
  carriers in `<slug>/work` folded, added under `listed` in
  `docs/project/index.yaml`, and rendered into `AGENTS.md` at delivery
  (`sdlc/procedure/lifecycle/integrate`).
- `verification/` stack (`Dockerfile.verify`, `docker-compose.verify.yml`,
  `seed/`) and CI workflow wiring when warranted by the project's archetype,
  verified live from clean state.
- When invoked inline by an admitted `builder` during a todo lifecycle, that
  `builder` commits the verified `verification/` stack and CI workflow files in
  `<slug>/work` (escalating any Layer 1 / Layer 2 contract delta back through
  preparation) and releases its workspace attempt via
  `instrukt todo stamp {slug} --build-complete`.

## Recovery

- If a local probe fails due to host resource limits, nested container
  isolation (Docker-in-Docker), or missing host tooling (for example, Docker
  daemon unavailable or iOS Simulator runtime not installed), route the probe
  to the repository's CI runner via a pushed branch/`pull_request` run when
  configured, or record the exact host prerequisite in
  `verification-substrate.md` (via preparation when inside a tracked todo) and
  report it plainly.

## See Also

- [sdlc/policy/live-verification](../policy/live-verification.md)
- [sdlc/procedure/lifecycle/work/verify](lifecycle/work/verify.md)
- [sdlc/procedure/project-analysis](project-analysis.md)
- [sdlc/procedure/greenfield-intake](greenfield-intake.md)
