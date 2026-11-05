---
description: "instrukt owns no toolchain: a project declares its own quality commands as named actions in instrukt.yaml, and procedures call them by name."
visibility: 'public'
---

# Project Toolchain Integration — Design

## Purpose

instrukt owns no testing, linting or formatting. Every quality step runs on the
project's **own** machinery (its `Makefile`, configs, tools). Nothing is copied
into the project. **The only plug is the project's `actions`** in its
`instrukt.yaml`: a name plus the project's own command (`test: make test`). The
flavor's procedures call an action by name; hooks never run one. Every tool and
config behind an action is detected and plugged onto, never prescribed. A project
stays installable and testable with no instrukt present.

## Inputs/Outputs

**The project owns** its commands, the per-language configs and manifests, and
the `actions` that name them:

| Action    | Does                                                                                                                                                                                   |
| --------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `install` | provisions every present toolchain: `uv sync` (python); corepack + lockfile-detected package manager (pnpm/npm/yarn/bun) + frozen install (node, incl. frontend); `cargo fetch` (rust) |
| `lint`    | lints whatever is present                                                                                                                                                              |
| `test`    | runs every suite present (python + frontend + rust)                                                                                                                                    |
| `format`  | formats whatever is present                                                                                                                                                            |

**Blessed defaults** — configs the flavor _offers_ (never prescribes) to seed a
greenfield setup or raise a brownfield one that is missing or weaker. The agent
detects what a project uses and plugs it onto its actions; the steward offers a
default where it helps. Tools are never mandated.

- **Python** — uv · ruff / pyright / mypy · pytest
- **TypeScript / node** — pnpm · tsc (typecheck) · vitest (test) · and a linter/formatter the agent picks by heuristic: **ESLint + Prettier** for Next.js/React (framework + React-Compiler lints) or **Biome** for framework-agnostic projects — both shipped as defaults. Frontend deps via corepack + lockfile-detected manager.
- **Rust** — cargo fmt / clippy / test

**instrukt offers, never copies in:** the flavor's procedures that call the
actions by name, and the offered skeleton defaults (the steward).

**Recipes delegate; they carry no inline logic.** A `Makefile` target is a thin
convention name: its recipe is a single delegation to a script or typed module
entrypoint that owns the behavior (a `tools/`/`bin/` bash script when it only
shells out to binaries, a module entrypoint when it earns a test). Projects are
offered the convention, never mandated — see `sdlc/policy/no-inline-logic`,
which governs every file that can carry an executable body.

## Invariants

1. The project's `actions` are the **only** plug; the tools and configs behind
   them are detected and offered, never prescribed.
2. instrukt tooling is never copied into a project; a procedure calls an action
   by name, and the action runs the project's own command.
3. A project remains installable and testable standalone through its own
   commands, with no instrukt present.
4. instrukt embeds no toolchain and adds no quality hooks and no levels; no hook
   runs an action.
5. The `install` action is the prerequisite for the other actions.
6. The `test` action is never file-scoped — the spec-anchored model has no 1:1
   source→test map.

## Primary flows

A procedure names the action; the agent reads its command with
`instrukt config get actions.<name>` and executes that command. For actions
declaring file support, files are appended as arguments. The `test` action
remains unscoped:

- `lint` → the project's lint command.
- `test` → the project's test command (unscoped).
- `format` → the project's format command.

Spec coverage is not an action: it runs through `instrukt specs coverage`, and
scenario reservation through `instrukt specs reserve`.

## Failure modes

- **Action not declared.** The procedure has nothing to call — it reports the
  missing action and points at the steward
  (`sdlc/procedure/code-config-assessment`) to declare it.
- **Toolchain not installed.** The `install` action provisions it. A project
  that runs `test` before `install` fails on missing binaries — that is the
  prerequisite ordering, not a defect in the action.
- **Project diverges from a default.** The steward raises the floor and respects the
  ceiling: it offers a default where the project is **missing or weaker**, leaves
  **deliberate, project-owned** config untouched, and asks the human only on a genuine
  conflict — never silently overwrites intent.
