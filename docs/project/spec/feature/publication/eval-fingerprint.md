---
description: "The eval fingerprint — a digest of every published snippet that declares `eval: true`, recomputed by every sync into the home's state, so that every stamp can name the doctrine it was produced under."
---
# Eval Fingerprint — Spec

## What it is

A snippet that declares `eval: true` in its frontmatter is part of the
doctrine whose effect is evaluated. The eval fingerprint identifies the
doctrine in force on this computer: a digest over those snippets. Every sync
that publishes recomputes it into `~/.instrukt/state/eval-fingerprint/`,
recorded per published commit. The current fingerprint is available to work;
stamp copying and reporting belong to
`project/instrukt/spec/domain/work`. Eval has no module and no switch; the fingerprint
is always computed.

## Canonical fields

### Shipped process set

The flavor marks prepare, build, review and quality-lane procedures and policies
with `eval: true`. Shared lifecycle, orchestration, commit and documentation
rules serving those phases are included. Coordinator, integrator, monitoring
and the report procedure itself are outside this shipped process set.

### The fingerprint

<!-- planned-change:compressed-global-entrypoints -->
- **Input.** Every snippet in the published corpus of the home — published
  from `docs/general/` or a `docs/domain/<x>/` of any project — whose
  frontmatter declares `eval: true`, after this sync's patch.
- **Determinism.** The same set of snippets with the same content yields the
  same fingerprint, whichever project's sync computed it and in whatever
  order the projects published.
- **Change.** Adding, removing or changing the content of an `eval: true`
  snippet changes the fingerprint; changes to other snippets do not.
- **Empty.** A corpus with no `eval: true` snippet has a defined fingerprint
  of its own, distinct from any non-empty one.
- **Association.** State records the fingerprint per published commit, rather
  than only overwriting a current digest. The current fingerprint remains
  available to work; no separate eval log is maintained.
- **When.** Computed by every sync that publishes, as part of the same
  publication, from the corpus as it stands then. A record is written, with
  the sync's time, only when what it holds differs from what was computed:
  the per-commit record when its fingerprint or snippet count differs, the
  current record when its fingerprint differs. A record whose content is
  unchanged is left as it is, its time included. That a commit was published
  before never decides it: another project may have changed evaluated
  doctrine since, which changes the fingerprint of an unchanged commit.
<!-- change:compressed-global-entrypoints -->
- **Input.** Every snippet in the published corpus of the home — published
  from `docs/general/` or a `docs/domain/<x>/` of any project — whose
  frontmatter declares `eval: true`, after this sync's patch.
- **Determinism.** The same set of snippets with the same content yields the
  same fingerprint, whichever project's sync computed it and in whatever
  order the projects published.
- **Change.** Adding, removing or changing the content of an `eval: true`
  snippet changes the fingerprint; changes to other snippets do not.
- **Compressed units.** The fingerprint also covers the current compressed
  units (`project/instrukt/spec/feature/publication/compressed-units`) that
  the global agent files hold, those of the always-on doctrine and of each
  runtime's own rules, and the current unit of each `eval: true` snippet. A
  stale unit is held by no rendering and does not enter it. Adding, removing
  or changing a covered unit changes the fingerprint, and what state records
  per published commit tells a change of evaluated prose apart from a change
  of covered units alone.
- **Empty.** A corpus with no `eval: true` snippet has a defined fingerprint
  of its own, distinct from any non-empty one.
- **Association.** State records the fingerprint per published commit, rather
  than only overwriting a current digest. The current fingerprint remains
  available to work; no separate eval log is maintained.
- **When.** Computed by every sync that publishes, as part of the same
  publication, from the corpus as it stands then. A record is written, with
  the sync's time, only when what it holds differs from what was computed:
  the per-commit record when its fingerprint or snippet count differs, the
  current record when its fingerprint differs. A record whose content is
  unchanged is left as it is, its time included. That a commit was published
  before never decides it: another project may have changed evaluated
  doctrine since, which changes the fingerprint of an unchanged commit.
<!-- /planned-change:compressed-global-entrypoints -->

## Use cases

The sync scenarios in
`project/instrukt/spec/feature/publication/project-publication` exercise
fingerprint computation as part of publication.

## Known caveats

<!-- planned-change:compressed-global-entrypoints -->
- Only published snippets (from `docs/general/` and `docs/domain/<x>/`) with `eval: true` enter the fingerprint; `docs/project/` snippets do not.
- The digest hashes committed raw snippet content, before read-time expansion.
<!-- change:compressed-global-entrypoints -->
- Only published snippets (from `docs/general/` and `docs/domain/<x>/`) with `eval: true`, and the compressed units the global agent files hold, enter the fingerprint; `docs/project/` snippets and their units do not.
- The digest hashes committed raw snippet content and the bodies of the units it covers, before read-time expansion.
<!-- /planned-change:compressed-global-entrypoints -->

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/feature/publication/project-publication](project-publication.md) — the sync that computes it.
