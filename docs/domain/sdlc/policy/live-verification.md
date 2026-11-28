---
eval: true
description: 'Live behavioral verification architecture: three-layer model (persistent substrate, pruned incident triggers, todo-scoped plan), system-archetype oracle routing, CI-first portable execution host, black-box driver and anti-cheat checker separation, publication boundary, and escape-driven tightening.'
visibility: public
---

# Live Verification — Policy

## Required reads

- [general/principle/verification-independence](../../../general/principle/verification-independence.md)
- [sdlc/policy/testing](testing.md)
- [sdlc/concept/defect-evidence](../concept/defect-evidence.md)
- [sdlc/policy/state-storage](state-storage.md)
- [sdlc/policy/security-awareness](security-awareness.md)
- [sdlc/policy/ci-pipeline-conventions](ci-pipeline-conventions.md)

## Rules

### Three orthogonal layers

- **Layer 1 — Verification Substrate (`project/<project>/spec/verification-substrate`).** Persistent project specification declaring the project's system archetype(s), real observable surfaces, execution host, isolation primitive and concurrency cap, deterministic boot and state-seeding entrypoints, content-addressed verification image contract where the archetype needs an isolated container stack, org-level sandbox credential-pool references (secret or proxy names only, omitting secret values), and explicitly declared blind spots. Layer 1 changes only when a delivery adds, removes, or alters a surface, runtime dependency, or test identity, carried through pending documentation carriers (`general/procedure/doc-snippet-authoring`) and folded at delivery.
- **Layer 2 — Reliability Triggers (`project/<project>/policy/reliability-checklist`).** Persistent catalog of diff-triggered live verification checks (`<semantic diff-trigger> -> <mandatory live check>`). Every entry originates from a confirmed real occurrence with an inspectable source address (`sdlc/concept/defect-evidence`). Agent intelligence evaluates which triggers fire against a candidate diff (`sdlc/policy/no-semantic-validators`). Layer 2 follows a **Promote -> Automate -> Prune** lifecycle: when a trigger recurs and can be locked into a deterministic or hybrid regression test, graduate it into code and delete the prose trigger; delete triggers whose subsystem is retired.
- **Layer 3 — Todo-Scoped Verification Plan (`verification-plan` todo artifact).** Derived dynamically per todo from approved intent, the actual candidate diff, Layer 1, and Layer 2. Selects the verification tier, cheapest honest oracle, target flows, adversarial perturbations, firing Layer 2 triggers, and concurrency. Retained only on that todo's work record and never promoted into Layer 1 or Layer 2; only structural substrate deltas and confirmed escape triggers graduate into standing project documentation.

### System-archetype oracle routing

Select the cheapest honest real-surface oracle matching the changed runtime or behavioral surface:

| Archetype / Change Class | Real Surface | Cheapest Honest Oracle |
| --- | --- | --- |
| Web app / SaaS / internal UI | Browser (Chromium + WebKit, desktop + mobile viewports) over live HTTP/DB/cache/auth | Isolated Compose stack on CI runner + black-box browser driver + anti-cheat checker + post-run DB/state inspection |
| Mobile app | iOS Simulator / Android Emulator | Booted simulator/emulator on CI or native host + black-box device driver + checker + screenshots/recording |
| Public API / backend service / webhooks / workers | Real HTTP/WS/queue endpoints backed by real DB/cache/local protocol emulators | Isolated Compose stack on CI runner + black-box protocol/concurrency driver + DB/log checker |
| CLI / local daemon / orchestrator | Compiled candidate binary, real git hooks, running daemon, local locks/files, relocatable target runtimes | Deterministic commands: real-binary functional tests and snapshots (`sdlc/policy/testing`). Concurrency, hooks, installation, or runtime-boundary behavior: isolated temp-home live-daemon / candidate-binary smoke |
| ERP / business-ops tenants | Real tenant state, ledger, stock, board, or GitOps reconciliation | Ephemeral/staging tenant + black-box workflow driver + independent native state/ledger proof |
| Libraries / parsers / compilers / data pipelines | Input corpus to output and algebraic invariants | Property-based testing, differential testing against reference/prior release, and sample corpus replay |

### Execution host and portability

- Containerized, browser, and mobile-simulator verification runs execute by default on an **ephemeral CI runner** (using the organization's subscription-backed agent runtime and reusable workflows per `sdlc/policy/ci-pipeline-conventions`), rather than nested Docker-in-Docker inside company containers or local workstations.
- Containerized verification stacks remain standard OCI and Compose definitions so compute stays portable across hosted CI runners and self-hosted or alternative runners via runner selection alone.
- Docker-free archetypes (isolated temp-home CLI/daemon smokes, real-binary tests, and property/corpus suites) execute cleanly in CI or locally without container privileges.

### Dynamic verification tiering

Classify every clean candidate HEAD by runtime or behavioral surface risk and blast radius:

- **Tier 0 (no live run warranted):** A prose-only change (`general/principle/intelligence-boundaries`), a pure type/formatting refactor, or a deterministic code change fully verified by compiler + real-binary functional tests + static review. Any accompanying change to software behavior is classified on its own surface. Write `verification-findings` citing the exact candidate HEAD SHA and reason, record the Tier 0 skip disposition for that HEAD, and skip live `verifier` execution.
- **Tier 1 (localized runtime surface change):** Single isolated Setup -> Driver -> Checker pass exercising the touched surface plus every firing Layer 2 trigger.
- **Tier 2 (high blast radius, multi-flow feature, external integration, or pre-release):** Parallel isolated fan-out across distinct flows (bounded by Layer 1's concurrency cap), full firing Layer 2 sweep, and captured visual/trace evidence.

### Two planning moments

- **Upstream substrate readiness (`Prepare`):** Before implementation begins, compare planned surfaces against Layer 1 and include any required Layer 1 pending update, verification-stack/seeder update, or declared blind spot inside the implementation plan.
- **Downstream dynamic plan (`Post-Build`):** After the candidate commit exists, derive Layer 3 from approved intent, the actual git diff, Layer 1, and Layer 2.

### Orthogonal dual verification and three-stage separation

Live verification (`verifier`) operates as the black-box dual of static white-box `build-reviewer` against the exact committed candidate HEAD before finalization, separated into three stages:

- **Setup (deterministic, white-box permitted):** Boots an isolated slot on the declared execution host and runs deterministic seed shortcuts to reach precondition state.
- **Driver (strictly black-box):** Runs in a fresh session given only the booted surface address, seeded credentials/role, and user-goal mission brief — excluding source code, git diffs, and internal helper paths. Interacts exclusively through the real user/runtime surface, captures raw evidence (recordings, screenshots, transcripts, observed state) on the authorized evidence surface, and reports both goal pass/fail and an unprompted anomaly list of anything unexpected or broken encountered during the run.
- **Checker (independent audit):** Reads both the Driver's raw evidence and the actual diff/intent. Audits Driver honesty (confirming real surface interaction without JS-eval bypasses, internal-endpoint shortcuts, or mock downgrades), verifies approved intent and every firing Layer 2 trigger, and triages anomalies into blocking in-scope defects (routed back to a `builder` remediation round) versus pre-existing unrelated defects (recorded as observations).
- **Standalone CI workflow bootstrap or path-migration rule:** Introducing a repository's first CI verification workflow (`sdlc/procedure/verification-bootstrap`) or renaming/replacing an established verification workflow at a new file path (which lacks a default-branch `workflow_dispatch` identity until merged) is always a standalone prerequisite delivery carrying zero application-feature delta. Its honest live oracle is the branch/PR checked-in baseline smoke exercising the shared verification entrypoint (image build/pull, boot, seed, one real surface end-to-end flow, and teardown at the exact candidate HEAD SHA), substituting for a per-todo Layer 3 code-blind Driver, audited by the independent host Checker from private raw CI evidence alongside independent static review of the workflow and `workflow_dispatch` interface (explicitly recording that `workflow_dispatch` on a new file path executes only once on the default branch).

### Autonomous bootstrap, content-addressed environments, and publication boundary

- Agents autonomously bootstrap and update Layer 1, verification stacks, and deterministic seeders from repository manifests without manual first-run prototyping (`sdlc/procedure/verification-bootstrap`).
- External dependencies resolve first to protocol-faithful local emulators (DB, cache, object store, mail, local OIDC, webhook emulators) and second to a pre-provisioned org-level sandbox credential pool (`instrukt secrets` or CI secrets). Human intervention is limited to supplying credentials for a brand-new external paid or legal third-party account requiring human billing, 2FA, or org OAuth consent into the pool.
- Verification container images are tagged and pulled by content hash of their build definition and dependency lockfiles (`<registry>/<repo>-verify:<sha256-prefix>`), instead of floating `:latest` tags. Parallel slots isolate state via distinct slot namespaces, ephemeral volumes, and dynamic ports, torn down cleanly on exit (`general/policy/shared-machine-resources`).
- **Storage and publication boundary (`sdlc/policy/state-storage`, `sdlc/policy/security-awareness`):** Structured text artifacts (`verification-plan`, `verification-findings`, `escape-assessment`) are stored on the todo work record with the exact candidate HEAD SHA recorded in the artifact text. Every verification outcome is a verification disposition the engine records as a stamp (`instrukt todo stamp <slug> --verification pass|fail|blocked|tier_0_skip|hotfix_deferred`), bound to the exact candidate commit and the current `verification-findings` report; a rewritten report or a new candidate commit leaves the earlier disposition as history and carries none until one is recorded for it. The stamp carries no raw evidence. Because todo artifacts reflect to issue trackers (including public repositories), reflected artifacts carry only sanitized, publishable summaries and authorized evidence references — omitting machine-local absolute paths, raw full boot logs, unredacted payloads/headers, and sandbox credentials. Heavy raw evidence stays outside Git and visible only to authorized reviewers:
  - On **private repositories**, raw evidence may reside in repository CI workflow artifacts accessible only to authorized collaborators (governed by CI retention TTL); `verification-findings` names the run whose artifacts hold it.
  - On **public repositories**, raw evidence (unredacted transcripts, screenshots from credentialed flows, raw boot or DB dumps) is never uploaded to public repository CI artifacts. When executing the **trusted default-branch verification workflow**, the trusted workflow may run the independent Checker inside the ephemeral CI job before teardown and upload only the sanitized, publication-safe verdict and redacted summary (or store raw evidence in private authorized storage). Whenever a public-repository candidate **authors or modifies the CI verification workflow itself** (requiring the independent host Checker to audit raw CI run evidence directly rather than trusting a candidate-run in-job summary), raw evidence from that CI run must be handed off through private authorized storage outside the public repository; absent that private storage, verification returns `BLOCKED` (a local probe credits only local behavior and never substitutes for required CI workflow evidence).
  - On **local runs**, raw evidence lives in the run's slot: a temporary location outside the worktree that the project's own test harness or its language's managed temporary-directory facility creates and removes when the process holding it ends. The Checker inspects it before that process ends, nothing of it is kept afterwards, and no agent deletes it: ending the holding process is the teardown.

### Escape ratchet

Every `bug` todo originating from a real occurrence (`sdlc/concept/defect-evidence`) records an `escape-assessment` artifact before finalization answering why pre-merge verification missed that defect class. It resolves to one disposition:

- `trigger-added`: add one diff-triggered entry to Layer 2 (`project/<project>/policy/reliability-checklist`) via a tracked follow-up todo when the bug fix restores existing documented behavior (preserving minimal bug-fix scope under `sdlc/policy/definition-of-done-lanes`).
- `substrate-upgraded`: deliver or file a tracked follow-up todo for the Layer 1 or stack upgrade closing the missing surface, engine, or test identity (filed as a tracked follow-up when the bug fix restores existing documented behavior).
- `regression-graduated`: bind a permanent deterministic or hybrid regression scenario/test within the bug delivery (`sdlc/policy/testing`).
- `escape-unmodelable`: state the concrete reason the external anomaly cannot be prevented upstream, or that the defect lay in prose, which review corrects (`general/principle/intelligence-boundaries`).

## Rationale

- Splitting persistent substrate, pruned escape memory, and todo-scoped planning avoids both static-checklist rot (accretion and blind spots on new surfaces) and pure dynamic amnesia (re-inventing boot recipes and forgetting past incidents).
- Routing by system archetype prevents verifying plumbing or running heavy browser sandboxes on deterministic CLI logic or prose while ensuring live runtime surfaces meet an honest oracle.
- Executing containerized and simulator verification on ephemeral CI runners eliminates Docker-in-Docker hazards for containerized company agents and workstation resource exhaustion, while standard OCI/Compose definitions preserve one-line portability to self-hosted runners if hosted CI limits tighten.
- Checking substrate readiness upstream during Prepare prevents build from stalling on missing verification infrastructure; deriving Layer 3 post-build covers actual unintended blast radius and firing incident triggers.
- Separating Setup, a code-blind Driver, and an anti-cheat Checker operationalizes `general/principle/verification-independence` on live behavior: a Driver that has never seen the diff cannot mirror builder assumptions or silently route around broken surfaces.
- Keeping raw evidence off reflected todo artifacts and public repository CI artifacts prevents leaking private paths, payloads, screenshots, or credentials to public readers.
- Content-addressed verification images (`<dockerfile-hash>`) guarantee reproducible verification across concurrent branches and historical replays, whereas floating `:latest` silently drifts.
- Wiring the Escape Ratchet directly to real `sdlc/concept/defect-evidence` occurrences turns every production escape into a durable tightening of the verification layer (`general/principle/evolution`) without letting speculative checks accrete.

## Scope

All software repositories using the `sdlc` workflow flavor across every system archetype. Projects without a Layer 1 substrate still apply dynamic tiering and the archetype matrix using their existing runtime surfaces until bootstrapped.

## Enforcement

The work record accepts verification dispositions through a neutral stamp requiring a current `verification-findings` artifact (and an open tracked `--follow-up` todo for `hotfix_deferred`). `--finalize-ready` enforces that the exact candidate commit carries a current handoff-admitting disposition (`pass`, `tier_0_skip`, or `hotfix_deferred`) matching the current `verification-findings` digest, and that a `pass` comes from an independent session distinct from the claim holder, drivers, and commit writers (`sdlc/procedure/lifecycle/overview`). Procedures assign role responsibilities: `todo-orchestrator` classifies candidate heads, routes `verifier` as a shared child of `<slug>/work`, and ensures `escape-assessment` is recorded on real-occurrence bugs; only a `verifier` records `pass`, `fail`, or `blocked`, `verifier` or `todo-orchestrator` records `tier_0_skip`, and `todo-orchestrator` records `hotfix_deferred`. A new head from `finalizer`'s upstream synchronization carries no disposition until one is recorded for it (`sdlc/procedure/lifecycle/work/finalize`). `planner` checks Layer 1 substrate readiness at prepare time (`sdlc/procedure/lifecycle/prepare/plan-draft`). Independent `build-reviewer` and `plan-reviewer` apply this policy to Layer 1/Layer 2 snippet changes and verification-stack definitions.

## Exceptions

- Emergency production hotfixes preserve every existing `sdlc/policy/definition-of-done` control (including static review, CI, and applicable browser-smoke checks) and may defer only the additional Tier 1 or Tier 2 live `verifier` pass — together with post-hotfix `escape-assessment` when the hotfix is a real-occurrence bug — by recording a hotfix deferral disposition (`--verification hotfix_deferred --follow-up <slug>`) for the exact candidate commit, naming the explicit tracked follow-up todo that delivers both, beside `verification-findings` carrying `Verdict: HOTFIX_DEFERRED`.

## See Also

- [sdlc/procedure/lifecycle/work/verify](../procedure/lifecycle/work/verify.md) — composable `verifier` role executing Tier 1/2 live verification.
- [sdlc/procedure/verification-bootstrap](../procedure/verification-bootstrap.md) — autonomous project substrate bootstrap procedure.
- [sdlc/design/lifecycle/work](../design/lifecycle/work.md) — work-phase iterative loop design.
