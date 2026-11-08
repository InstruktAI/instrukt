---
description: "instrukt owns no toolchain: procedures run the checks the project documents, and required CI checks guard integration."
visibility: 'public'
---

# Project Toolchain Integration — Design

## Purpose

instrukt owns no testing, linting or formatting. Every quality step runs on the
project's **own** machinery (its `Makefile`, configs, tools). Nothing is copied
into the project. Procedures read the project's documented commands and run
those checks. There is no project-layer `actions` key or separate command
registry. Every tool and config behind a command is detected and used, never
prescribed by instrukt. A project
stays installable and testable with no instrukt present.

## Inputs/Outputs

**The project owns** its commands, the per-language configs and manifests, and
their documentation:

| Command purpose | Does                                                                                                                                                                                   |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `install` | provisions every present toolchain: `uv sync` (python); corepack + lockfile-detected package manager (pnpm/npm/yarn/bun) + frozen install (node, incl. frontend); `cargo fetch` (rust) |
| `lint`    | lints whatever is present                                                                                                                                                              |
| `test`    | runs every suite present (python + frontend + rust)                                                                                                                                    |
| `format`  | formats whatever is present                                                                                                                                                            |

**Blessed defaults** — configs the flavor _offers_ (never prescribes) to seed a
greenfield setup or raise a brownfield one that is missing or weaker. The agent
detects what a project uses and documents how to run it; the steward offers a
default where it helps. Tools are never mandated.

- **Python** — uv · ruff / pyright / mypy · pytest
- **TypeScript / node** — pnpm · tsc (typecheck) · vitest (test) · and a linter/formatter the agent picks by heuristic: **ESLint + Prettier** for Next.js/React (framework + React-Compiler lints) or **Biome** for framework-agnostic projects — both shipped as defaults. Frontend deps via corepack + lockfile-detected manager.
- **Rust** — cargo fmt / clippy / test

**instrukt offers, never copies in:** the flavor's procedures that read the
project's documented checks, and the offered skeleton defaults (the steward).

**Recipes delegate; they carry no inline logic.** A `Makefile` target is a thin
convention name: its recipe is a single delegation to a script or typed module
entrypoint that owns the behavior (a `tools/`/`bin/` bash script when it only
shells out to binaries, a module entrypoint when it earns a test). Projects are
offered the convention, never mandated — see `sdlc/policy/no-inline-logic`,
which governs every file that can carry an executable body.

## Invariants

1. The project documents its checks beside its own tooling; procedures use
   that source rather than a second registry in instrukt configuration.
2. instrukt tooling is never copied into a project; procedures run the
   project's own documented commands.
3. A project remains installable and testable standalone through its own
   commands, with no instrukt present.
4. instrukt embeds no toolchain and adds no quality hooks and no levels; no hook
   runs a quality command on instrukt's behalf.
5. The documented installation command provisions checks before they run.
6. The test suite is never file-scoped — the spec-anchored model has no 1:1
   source→test map.

Required CI checks remain the merge gate; without CI, the integrator runs the
project's documented test command against the exact candidate.

## Primary flows

A procedure reads the project's documented checks and executes their commands
under their supported options. File scoping follows that documentation where
allowed; the test suite remains unscoped:

- `lint` → the project's lint command.
- `test` → the project's test command (unscoped).
- `format` → the project's format command.

Spec coverage keeps its own CLI: it runs through `instrukt specs coverage`, and
scenario reservation through `instrukt specs reserve`.

## Failure modes

- **Checks are not documented.** The procedure reports the missing project
  contract and routes assessment through `sdlc/procedure/code-config-assessment`.
  It never invents an empty check to make a gate green.
- **Toolchain not installed.** Provision it through the project's documented
  installation command before running checks.
- **Project diverges from a default.** The steward raises the floor and respects the
  ceiling: it offers a default where the project is **missing or weaker**, leaves
  **deliberate, project-owned** config untouched, and asks the human only on a genuine
  conflict — never silently overwrites intent.
