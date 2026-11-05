---
description: 'Base TypeScript project guidance: generator-first scaffolding, quality requirements, and the rulings on package managers and lint tooling.'
visibility: 'public'
---

# TypeScript Scaffolding — Procedure

## Goal

Establish the shared TypeScript ground rules every platform flavor builds on.
Flavors prescribe _qualities and buttons_; the framework's own generator owns
the files.

## Preconditions

- The workload is classified per the delivery portfolio and a platform flavor
  is chosen — this base is never scaffolded alone.
- Verify current generator/tooling state with a quick search; generators and
  flags churn faster than training data.

## Steps

1. **Scaffold with the framework's own generator, always** — the current
   generator invocation for the chosen flavor, aimed at
   the project's repository folder, where `instrukt init` has run (target `.`)
   rather than a new directory. A generator that refuses a folder already
   holding files it doesn't recognise runs in a temporary sibling folder with
   its own git setup turned off; its output, dot files included, then moves
   into the project folder, which keeps its repository and `instrukt.yaml`.
   Check the chosen generator's current rules for pre-existing files before
   invoking it in the project folder. Never deploy scaffold files from a blessed
   folder — a baked scaffold is a stale cache of the generator.

2. **Package manager ruling**: pnpm blessed for greenfield; in any existing
   repo the lockfile is law; never mix. The button contract is
   manager-agnostic, so this choice never leaks into scripts or CI.
3. **Lint/format ruling**: where the framework has a first-class lint
   ecosystem (Astro, Next.js), keep it (ESLint + Prettier). Where the repo is
   framework-neutral (Hono services, Vite tools, CLIs), use Biome as the
   single lint+format tool. The qualities are non-negotiable either way: one
   formatter, strict TypeScript (`strict`, `noUncheckedIndexedAccess`), types
   checked in CI.
4. **Standard scripts exist regardless of flavor**: `dev`, `build`, `lint`,
   `format`, `typecheck`, `test` — plus the flavor's additions. A thin
   Makefile shim maps `lint`/`format`/`test`/`e2e` onto them; `instrukt.yaml`
   declares those targets as `actions`, and shared CI calls them (see the CI
   pipeline conventions policy).
5. **Quality gates are wired before feature work**, per the applicable
   production baseline.

## Outputs

- A generator-produced repo meeting the qualities above, with the button set
  and Makefile shim in place.

## Recovery

- If generator output conflicts with these rules, prefer the generator's
  ecosystem convention and satisfy the _quality_ another way; record the
  deviation in the repo's AGENTS.md.

## See Also

- [sdlc/concept/delivery-portfolio](../../concept/delivery-portfolio.md)
- [sdlc/policy/ci-pipeline-conventions](../../policy/ci-pipeline-conventions.md)

- The chosen framework generator's current documentation and source — verify its rules for pre-existing files against the version in use.
