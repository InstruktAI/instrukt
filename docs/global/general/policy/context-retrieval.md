---
description: 'Guidelines for AI agents to retrieve relevant documentation snippets on demand.'
visibility: 'public'
---

# Context Retrieval Policy — Policy

## Rules

- **Read a domain's rendering once, at entry.** When a task enters a knowledge
  domain, run `instrukt docs get --domain <domain>` and read its output in
  full, unless that domain's contracts are already loaded this session. A task
  enters `sdlc` when it writes or changes code or tests. The rendering is the
  domain's mandatory contract in one pass: its `expanded` snippets with their
  closure, its `listed` snippets as one line each, and its triggered
  procedures. Your global agent file names each published domain with that
  command; a snippet that always needs a domain names it in its
  `## Required reads` by its bare name.
- **`instrukt docs` is the entry point for discovery, and always filtered.** Use
  `--domain`, `--areas` and the other filters; an unfiltered index floods the
  context. Retrieve before any task where missing context could alter
  decisions.
- **Three phases:** filtered index, then `instrukt docs get <id>` for the selected
  ids, then the ids listed under `# Required reads (not loaded)` in that
  response. Dependencies are never expanded inline by default; `--closure`, on
  any snippet, takes them in the same pass.
- **A listing line names the command to run.** A triggered procedure's line
  names `instrukt docs get <id> --closure`, which delivers the procedure with
  its transitive required reads in one pass; a scheduled job's startup
  instruction does the same for the procedure its declaration names. A listed
  snippet's line names `instrukt docs get <id>`, which keeps the explicit
  follow-up flow.
- **Third-party docs: consolidated first.** Run `instrukt docs index --third-party`
  before web search or Context7; those are fallbacks for coverage gaps.

## Rationale

Immediate context prevents drift from established policy, architecture, and
procedure. The rendering, not the files behind it, is what an agent reads: it
leaves undelivered content out and resolves every reference, so the agent
grounds in the current contract. Consolidated third-party docs are curated and project-aware; live web
results are noisy and bypass shared knowledge.

## Scope

All agents, all projects, all tasks.

## Enforcement

Reviewers treat domain work begun without the domain's contracts loaded as a
process miss; the remedy is to load them before continuing.

## Exceptions

None.
