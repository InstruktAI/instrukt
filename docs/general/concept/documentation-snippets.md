---
description: 'Modular documentation system for selective context delivery to AI agents.'
visibility: 'public'
---

# Documentation Snippets — Concept

## What

Documentation snippets are atomic markdown files that each capture one idea — a principle,
concept, policy, procedure, design, spec, or decision record (ADR). Each snippet's type
and scope follow from its path, it follows the section shape its folder's schema gives
that type, and it can declare hard dependencies on other snippets.

Snippets live in repositories, in up to three positions:

<!-- planned-change:instrukt-first-build -->
- **General** (`docs/general/`) — published from committed main and rendered
  into every agent CLI's user-scope instruction file: always on.
- **Domain** (`docs/domain/<x>/`) — published from committed main; one line in
  every global agent file, and the full rendering through
  `instrukt docs get --domain <x>`: on demand.
- **Project** (`docs/project/`) — the project's own knowledge, living alongside
  what it describes, indexed from committed main and rendered into the project's
  root `AGENTS.md`. Retrieval reads its files in the registered primary checkout.
<!-- change:instrukt-first-build -->
- **General** (`docs/general/`) — published from committed main and rendered
  into every agent CLI's user-scope instruction file: always on.
- **Domain** (`docs/domain/<x>/`) — published from committed main; one line in
  every global agent file, and the full rendering through
  `instrukt docs get --domain <x>`: on demand.
- **Project** (`docs/project/`) — the project's own knowledge, living alongside
  what it describes, published from committed main to
  `~/.instrukt/docs/project/<slug>/…` and rendered into the project's root
  `AGENTS.md`. Retrieval reads the published copy.
<!-- /planned-change:instrukt-first-build -->

Each position's `index.yaml` selects what its rendering carries.

Every snippet has one qualified identity, the same from every location, so a
reader reaches any repository's snippets from anywhere by naming the project or the
identity.

Agents retrieve snippets on demand via `instrukt docs index` + `instrukt docs get`
rather than loading
everything upfront.

Every type except `adr` is a **living, present-tense contract** — it states what is
currently true (the shape, the rule, the behavior) and is rewritten as reality changes.
`adr` is the sole exception: a **frozen, dated, append-only decision _record_** capturing
_why_ a contestable choice was made. A `design` snippet is the _shape_ (what); an `adr`
is the _reasoning behind a decision_ (why). **Design is not a record, and an ADR is not
design.**

## Why

AI agents work within limited context windows. Bundling all documentation into every
prompt wastes tokens and dilutes signal. Snippets make knowledge modular and selectively
retrievable — an agent pulls only what it needs for the task at hand. The three positions
keep shared knowledge maintained once and published everywhere, domain knowledge out of
context until a task needs it, and each project's own knowledge beside its owner,
without duplication.
