---
description: Deterministic release checks of instrukt's consumed T3 surfaces and its test double, with a verified stable pin and unattended nightly warning.
visibility: public
delivered_by:
  - thin-live-runtime-boundary
---
# T3 Contract — Spec

## What it is

The T3 runtime adapter has a deterministic compatibility check against a real
headless T3 release. Maintainers receive a reproducible result for instrukt's
consumed interfaces, including the agreement of its test double with those
interfaces. CI checks the supported stable release on every change and the
newest nightly on an hourly schedule. Successful checks require no agent.

This contract applies the external-boundary discipline of `sdlc/policy/testing`
and the repository's bounded T3 exception in `project/instrukt/policy/testing`.
The runtime-neutral facade lives in `project/instrukt/spec/feature/runtime/adapter`,
and T3's operational realization — including exact project resolution, company
home project titling, mode resolution, and live RPC semantics — is owned by
`project/instrukt/spec/feature/runtime/t3`.

## Canonical fields

### Supported release and acquisition

`crates/instrukt-runtime-t3/t3.version` owns the default branch's verified
stable version, independently of the Graphify pin. Renovate tracks that file
with the `github-releases` datasource for `pingdotgg/t3code`, strips the tag's
leading `v`, accepts stable releases only, and uses the repository's green-check
PR automerge policy. This pin identifies instrukt's tested dependency; it
neither installs nor constrains the T3 version a person runs.

The CI runner acquires `t3-<version>-linux-x64.tar.gz` and `SHA256SUMS` from
the same release tag. Before extraction or execution it verifies the archive's
SHA-256 against that filename's checksum entry. Missing or ambiguous checksum
entries, a digest mismatch, unavailable assets, and execution failures fail the
check visibly. The downloaded executable is the sole T3 used by this check; an
installed executable never substitutes for it.

### Native launch, live projection, and project-table agreement

The consumed live native boundary includes `GET /api/projects`,
`server.getConfig`, `orchestration.getThreadProjection`,
`orchestration.launchThread`, and `orchestration.dispatchCommand`. The suite
verifies fixed command/thread identity and opening-message replay (including
replaying an accepted launch after thread deletion), captured native project,
model-selection options and resolved modes, existing-worktree capability, and
project-resolution/title agreement against `project/instrukt/spec/feature/runtime/t3`
where provider-free evidence permits. Native launch accepts `createdBy:"user"`
on the pinned WebSocket path; caller payload provenance does not override it.
Grammar or projection shape alone proves neither completed workspace
preparation nor successful provider execution.

The T3 test double holds its project table as explicit state (never deriving
projects from folders under test), and the agreement check compares the real
release and fake on `GET /api/projects`, `t3 project add`, and the titles
assigned to regular checkouts and company homes under
`project/instrukt/spec/feature/runtime/t3`.

### Consumed upstream surfaces

Paths in this table belong to `pingdotgg/t3code`; the instrukt identities name
the owners of the required behavior.

| Upstream surface | Instrukt contracts to check |
| --- | --- |
| Contracts package, `packages/contracts/` | Runtime operations, response shapes, and capability evidence in `project/instrukt/spec/feature/runtime/adapter` and `project/instrukt/spec/feature/runtime/t3`. |
| Server persistence migrations, `apps/server/src/persistence/Migrations.ts` and `apps/server/src/persistence/Migrations/`, and their projection writer, `apps/server/src/orchestration-v2/ProjectionStore.ts` | Strictly non-live read-only reads: cold text extraction (`project/instrukt/spec/feature/runner/history`) and thread-retention projection confirmation (`project/instrukt/spec/feature/runner/thread-retention`). No live operational path consumes these tables. |
| WebSocket contracts/handlers, native `ThreadLaunchService`, and project HTTP snapshot | Fixed native launch/replay, queued command dispatch, live thread projection observation (`orchestration.getThreadProjection`), captured project/model/modes, and existing-worktree placement in `project/instrukt/spec/feature/runtime/t3`. |
| CLI entry and commands, `apps/server/scripts/cli.ts` and `apps/server/src/cli/` | Unattended runtime operations and evidence in `project/instrukt/spec/feature/runtime/t3`. |

### Isolation and real evidence

Each check uses a temporary fixture directory, T3 base directory, instrukt home,
and Git project. Server startup uses an explicit base directory, loopback
address, and `--no-browser`; every client resolves that same base. Inherited T3
caller, connection, development-server, and credential settings are removed.
The check uses no person's application data or credentials, running T3, or model
execution; host environment and isolated application roots follow the owning
isolation contracts (`project/instrukt/spec/feature/configuration/logging` and
`project/instrukt/spec/feature/publication/agent-entrypoints`). All issued
sessions and all processes owned by the check are cleaned up on success,
failure, and interruption within bounded operation deadlines. Synchronization
uses observable readiness and changes under `sdlc/policy/testing`.

T3 itself creates and migrates the disposable database and writes the
projection records for the non-live cold text-read and thread-retention checks.
A schema fabricated by the test is not evidence of compatibility with T3's
migrations. Fixtures use provider-free management commands; a deferred run
remains unreleased throughout any queued-input probe. Non-live checks open T3's
database read-only through the normal source-discovery path; instrukt's test
never migrates or patches T3's database.

The check exercises the following evidence through the existing public adapter
interfaces and real CLI boundaries, without widening implementation visibility:

| Evidence | Required check |
| --- | --- |
| CLI grammar | Help accepts the consumed start/title, provider/model, worktree/branch, send, list/project/JSON, show/message-window, and session issue/revoke options. Grammar evidence is distinct from successful model execution. |
| Discovery and readiness | The real runtime descriptor, short-lived read-scoped token issue/revoke, and negotiated `server.getConfig` response pass production readiness acquisition and shape validation. Unready provider facts remain unready; no successful provider login or quota observation is invented. Revocation is checked, including denial of reuse of the revoked credential. The same response's provider instances decode into configured agent runtimes and their homes (`project/instrukt/spec/feature/publication/agent-entrypoints`). |
| Thread identity and state | Provider-free thread creation supplies real list, show, and `orchestration.getThreadProjection` responses. The adapter decodes identity, title/key, project, status, native activity, settled/archive, and available workspace metadata; the tested fixture determines expected values. Unknown-thread responses retain failure meaning. |
| Accepted input and idempotent dispatch | Authenticated WebSocket `orchestration.dispatchCommand` with a retained command ID and native CLI queue delivery are checked against a deferred run for queued acceptance, replay without a duplicate, native `createdBy:"user"` stamp, and live message/run observation via `orchestration.getThreadProjection` — verifying that the production live adapter completes without reading `statev2.sqlite`. |
| Non-live cold text source and thread retention | The cold text source opens the real migrated database read-only and validates the thread and message tables it consumes; `instrukt history extract` retains the fixture thread with its user messages and nothing else, and an unchanged second extraction loads nothing. Thread retention decodes finished, quiet, pinned, and latest-activity facts, validates its projection tables, and checks `orchestration.dispatchCommand` with `thread.archive` (including refusal of a second archive). |

Each disposable server start waits for the runtime descriptor to report that
server's assigned port before connecting, including after a restart.

The consumed expectations derive from the owning specs and adapter contract,
not from copying every field of an upstream response. Additional upstream
fields are acceptable where the adapter permits them. Release and migration
numbers are result diagnostics; descriptor, protocol, and consumed shape
determine adapter compatibility under `project/instrukt/spec/feature/runtime/t3`.

### Agreement of the fake with the real boundary

The same contract check exercises `crates/instrukt-cli/tests/support/t3.rs`
against the real CLI and RPC provider-free observations. Equivalent native
thread facts drive both. Normalization removes only incidental identities,
paths, ports, and timestamps, retaining field presence, types, state meaning,
input/run relationships, and success/failure meaning. A consumed-field or
behavior disagreement fails the check with the operation identified. Fake-only
fault-injection controls remain test controls; they make no claim about a T3
interface.

Every emulated operation has an explicit disposition in the result: verified
against the real boundary, fault injection, or outside provider-free coverage
under Known caveats. Passing the check never labels an unexercised success path
as verified. This is verification of this T3 fake, not a repository-wide fake
framework or a change to other test levels.

### Pinned and nightly CI

The `pinned T3 contract` job in `.github/workflows/ci.yml` runs the contract
suite on every push and pull request admitted by that workflow, including
dependency PRs, against the stable pin. It is a required green result for the
repository's dependency automerge. The `t3-contract` feature selects the
real-release check; missing prerequisites under that feature fail rather than
skip it. The ordinary hook and test commands retain their existing scope.

`.github/workflows/t3-nightly.yml` runs hourly on the default branch. It
resolves the newest version-ordered `v*-nightly.*` tag and its commit using
`git ls-remote --tags --refs`, without the releases API for discovery. It
compares that version with the pin recorded on the workflow-owned
`upstream/t3-nightly` branch. An unchanged nightly skips the update; the check
is dispatched again when no contract run exists for the recorded head, so a
dispatch that failed or was interrupted is repaired at the next run. A missing
branch or changed nightly produces a pin-bump commit on that branch, using the
current default-branch source with `crates/instrukt-runtime-t3/t3.version` set
to the nightly version. The default branch's stable pin and the person's
installation stay unchanged. The nightly branch is retained for
dependency-head observation and is not promoted or automatically merged into
the default branch.

After pushing the new head with `GITHUB_TOKEN`, the scheduled job explicitly
dispatches `.github/workflows/t3-contract.yml` with `ref: upstream/t3-nightly`.
That workflow is present on the default branch, declares `workflow_dispatch`,
and runs the same UC-TC1 suite using the dispatched branch's pin and source.
Its check belongs to the dependency commit, not the default-branch scheduler
commit. The workflow-owned branch uses the `upstream/` namespace, separate from
Renovate's branches. Concurrent nightly updates serialize; a changed branch
head is not overwritten without checking the expected previous head.

The scheduled job requires repository-token permissions to write the pin-bump
commit and dispatch Actions workflows (`contents: write`, `actions: write`).
It does not rely on a push trigger: GitHub suppresses push-workflow recursion
for pushes made with `GITHUB_TOKEN`, while `workflow_dispatch` is an explicit
exception. A failed push or dispatch fails the scheduled job visibly. A
recorded pin is not a claim that its check completed or passed; the contract
workflow also supports redispatch against the recorded branch after a failed
dispatch.

The result manifest records the evaluated instrukt revision, release tag,
upstream SHA when resolved, archive name and digest, exercised surfaces, fake
verification dispositions, exclusions, outcome, and operation-specific failure.
The workflow retains it and diagnostic output on failure as well as success;
credentials and conversation bodies are not diagnostics. Acquisition or harness
failure remains distinguishable from an incompatible T3 response.

### Existing dependency-failure route and prerequisites

Instrukt's own project enables `modules.events` in `instrukt.yaml` and selects
both `renovate/` and `upstream/` in `ci.dependency_branch_prefixes`. Its watch
list is empty. These are prerequisites for the existing dependency-failure
route for stable Renovate updates as well as nightly checks; a prefix does not
implicitly enable events. The enabled route needs its existing installed event
infrastructure, observation credentials, and integrator participant.

`project/instrukt/spec/feature/runner/event-activation` owns observation of the
current selected branch heads. Each new nightly pin produces a new commit, so
its failed contract check enters the existing `runner.dependency_failing`
route with that branch, head, check-run identity, and conclusion. Existing
occurrence deduplication remains unchanged. The branch's pin and retained
contract result identify the upstream release for the integrator's assessment
and warranted, deduplicated bug filing under
`project/instrukt/spec/feature/workflow/integration-gate`. A nightly failure
belongs to the dependency branch and does not report a broken canonical
delivery. Successful checks wake no T3 watcher.

## Use cases

### UC-TC1: A maintainer checks the T3 dependency without supervising an agent

```gherkin
Given an instrukt maintainer has a T3 release to evaluate
And no model account or personal runtime is available to the check
When the compatibility check evaluates that release
Then it exercises the supported integration against the real release in disposable state
And it checks that the emulated integration agrees on the exercised behavior
And it reports compatibility or failure with the affected operation identified
And it names every capability it could not exercise
And it leaves the maintainer's runtime and home untouched
```

## Known caveats

- This supplies warning, not a gate on T3's own updater. A nightly promoted
  within the hour can reach stable before its check finishes. Only the newest
  nightly at each observation is selected; intervening nightlies are not all
  enumerated. Scheduler delay and unavailable CI can lengthen the warning gap.
- Dependency observation sees current selected branch heads. A failure on a
  head replaced by a newer nightly before observation may be missed; retained
  Actions diagnostics do not provide catch-up through this route.
- A pin commit records the selected nightly, not completed verification. A
  dispatch that never produced a run is repeated by the next hourly run; a run
  that completed and failed is not repeated for an unchanged nightly.
- Provider-free checks cover migration shape and payloads actually produced by
  management and deferred-input operations. Successful model execution,
  assistant streaming/completion, reasoning/tool/diff payload generation,
  provider-created subagents, native child results, actual token/quota
  accounting, and authenticated provider readiness are not live exercised;
  history's retention of provider-produced assistant messages therefore remains
  outside this live guarantee.
- CLI start with an opening message starts execution; its successful model
  turn, ready-thread send/consumption, automatic wake after completion, and
  their fake success paths are not verified by provider-free checks.
- Reapplying current migrations to a fresh database does not prove every
  upgrade from a historical T3 database. Linux-x64 is the CI target; other
  release platforms require their own execution evidence.
- Without the enabled project event route, installed prerequisites, and a
  running participant, CI still produces results but automatic assessment and
  bug filing are unavailable.

## See Also

- [project/instrukt/spec/domain/runtime](../../domain/runtime.md)
- [project/instrukt/spec/feature/runtime/adapter](adapter.md)
- [project/instrukt/spec/feature/runtime/t3](t3.md)
- [project/instrukt/spec/feature/runner/history](../runner/history.md)
- [project/instrukt/spec/feature/runner/thread-retention](../runner/thread-retention.md)
- [project/instrukt/spec/feature/runner/event-activation](../runner/event-activation.md)
- [project/instrukt/spec/feature/workflow/integration-gate](../workflow/integration-gate.md)
- [project/instrukt/spec/feature/configuration/logging](../configuration/logging.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
- [project/instrukt/policy/testing](../../../policy/testing.md)
- https://github.com/pingdotgg/t3code/tree/bf469bb19669671d0df2efda74ed73867a7137c0 — inspected v0.1.8 source; CLI, RPC, migrations and projection writer are the upstream evidence.
- https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow — repository-token recursion rules, the dispatch exception and default-branch workflow registration.
- https://docs.github.com/en/rest/actions/workflows#create-a-workflow-dispatch-event — dispatch trigger declaration, branch/tag ref and Actions write capability.
