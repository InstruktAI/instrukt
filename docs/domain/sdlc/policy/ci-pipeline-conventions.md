---
eval: true
description: 'The two-tier Makefile button contract (fast agent loop vs heavy e2e lane) and how projects share CI: a shared workflows repository consumed by composition, deliberate divergence, and references that cannot move across organisations.'
---

# CI Pipeline Conventions — Policy

## Rules

### The two-tier button contract

- Every repo's Makefile exposes the **agent loop**: `lint`, `format`, `test`,
  `install` — fast by design, safe to run on every change. The project
  documents these commands in its project docs, and procedures run those checks.
  Required CI checks remain the merge gate.
- Every repo with heavy verification adds the mandatory **delivery lane**:
  `make e2e` — the suites too slow for the agent loop (browser/visual suites
  for sites, integration harnesses for services, release-grade test runs).
  CI is its only routine caller.
- CI composition is therefore universal and repo-blind: fast job runs
  `make lint test`, gate job runs `make e2e`, deploy runs after both.
- Frontend repos implement the Makefile as a thin shim delegating to their
  `package.json` scripts (see the frontend production baseline's button set);
  the Makefile stays the single cross-language interface.
- **Tier semantics.** `test` is the in-process functional suite — the agent
  loop, run on every change; a separate `test-functional` target must not
  exist, because `test` already means functional. `e2e` exercises the built
  or deployed shape through real interfaces and gates delivery. For sites,
  the smoke suite is the `e2e` implementation; the visual snapshot suite is
  a local instrument (platform-keyed baselines) and never a CI tier. Repos
  may add private targets (`test-agents`, `coverage`, …) freely — the
  contract mandates the two shared buttons, it does not forbid more.

### Shared workflows

- **CI has a shared home.** Generic pipeline logic lives in a shared workflows
  repository and is consumed from there. `InstruktAI/workflows` is the public
  reference for shared CI built on the button contract. A project looks in its
  organisation's shared repository before writing pipeline logic of its own.
- **Within one organisation, projects compose.** A project's own workflow files
  are the plumbing: they decide when CI runs, which permissions it holds and
  which inputs and secrets it passes on, and they take the generic work from the
  organisation's shared repository at a released version. A project may run its
  own jobs alongside or after the shared ones; that is composition, not
  duplication.
- **Divergence is deliberate and visible.** The default is the shared workflow
  as it is. When it does not fit, the improvement belongs in the shared
  repository, where every consumer gains it. A local variant is sometimes
  justified — an urgent fix, an experiment, an improvement on its way upstream —
  and is then declared as a divergence, with its reason and its way back.
- **Some pipelines belong to their project.** Where the logic expresses the
  project's own domain rather than the craft of CI — a deployment target that is
  itself what the pipeline operates, a release process no other project shares —
  the project owns that YAML outright. The test is whose knowledge the logic
  encodes.
- **Shared workflows keep their callers working.** A change aims to stay
  compatible with the projects already consuming it; a change that cannot ships
  as a new release that callers adopt deliberately. Only the shared repository's
  release process moves a release reference.
- **Across organisations, references do not move.** Another organisation's
  workflows and actions are consumed pinned to a reviewed full commit, and each
  update arrives as a reviewed change, which an automated review may carry. A
  shared workflows repository pins every third-party action it calls in the same
  way, so that a consumer's pin to it holds all the way down.
- **Pin by default; fork for a reason.** Forking another organisation's shared
  repository is justified when the project needs behaviour that does not belong
  upstream, when its organisation admits CI code only from inside the
  organisation, or when it must not depend on the upstream repository's
  continued existence. A fork made to propose a change upstream is a
  contribution, and consumption returns to the pinned upstream once the change
  lands. A fork becomes that organisation's own shared repository, maintained
  deliberately.
- **Hand over only what is needed.** A shared workflow receives the secrets and
  permissions its job requires, never all of them by default.

## Rationale

`make test` must stay fast or agents stop running it; heavy suites must gate
releases or regressions ship. Two named tiers keep both true and give CI one
standardized entry across every language and flavor.

Pipeline logic copied into each project drifts, and a fix made in one copy
reaches none of the others; a shared home makes every improvement everyone's.
A blanket ban on local variants would push urgent fixes and experiments into
unacknowledged copies, so divergence is allowed and made visible instead, which
keeps the way back open. Logic that encodes one project's domain gains nothing
from sharing and would make the shared repository carry knowledge it does not
own.

One change to a shared workflow reaches every project consuming it at once,
including projects that never asked for it. Keeping changes compatible, and
shipping the ones that cannot be as a release callers adopt deliberately, keeps
a change made for one project from breaking unrelated projects without warning.

A shared workflow runs inside its caller, with the caller's token and every
secret passed to it, so whoever can move a reference controls code running on
those secrets. Within an organisation that publisher is the organisation
itself, and the release process guarding its references is the safeguard.
Across organisations the publisher is a stranger, and a tag or branch can be
moved to other code at any time; a reviewed full commit is the only reference
that cannot. Pinning keeps upstream improvements reachable at the cost of
reviewing each update, while a fork trades that review for maintaining a copy,
which is why a fork needs a reason of its own.

## Scope

All repos with CI, all flavors. The `e2e` target is mandatory wherever a
delivery gate exists (every deployed site and service); pure libraries and
scripts may omit it.

## Enforcement

- A deployed repo whose CI does not run `make e2e` (or whose Makefile lacks
  the target) fails review.
- Composition and divergence are judged by the paradigm-fit lane's
  existing-path check (`sdlc/policy/definition-of-done-lanes`).
- References, secrets and permissions handed across organisations are judged
  by the security lane.

## Exceptions

- The shared workflows repository owns its own YAML outright.

## See Also

- [sdlc/design/project-toolchain](../design/project-toolchain.md) — the documented project checks this extends.
- [sdlc/policy/frontend-production-baseline](frontend-production-baseline.md) — the frontend script buttons behind the shim.
