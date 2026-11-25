---
description: "The eval fingerprint — derived from a prose digest over every published `eval: true` snippet and every always-on source, and a units digest over the always-on sources' current compressed units, recomputed by every sync into the home's state, so that every stamp can name the doctrine it was produced under and a change of prose is told apart from a change of compression alone."
---
# Eval Fingerprint — Spec

## What it is

A snippet that declares `eval: true` in its frontmatter is part of the
doctrine whose effect is evaluated. The always-on doctrine every agent starts
with is part of it as well, together with the compressed units the global
agent files hold for it
(`project/instrukt/spec/feature/publication/compressed-units`). The eval
fingerprint identifies the doctrine in force on this computer. It is derived
from two digests: a prose digest over the evaluated snippets and the
always-on sources, and a units digest over the always-on sources' current
units. Every sync that publishes recomputes them into
`~/.instrukt/state/eval-fingerprint/`, recorded per published commit. The
current fingerprint is available to work; stamp copying and reporting belong
to `project/instrukt/spec/domain/work`. Eval has no module and no switch; the
fingerprint is always computed.

## Canonical fields

### Shipped process set

The flavor marks prepare, build, review and quality-lane procedures and policies
with `eval: true`. Shared lifecycle, orchestration, commit and documentation
rules serving those phases are included. Coordinator, integrator, monitoring
and the report procedure itself are outside this shipped process set.

### The fingerprint

- **Input.** After this sync's patch, from what the home publishes from
  `docs/general/` and the `docs/domain/<x>/` positions of any project:
  - the evaluated snippets: every published snippet whose frontmatter
    declares `eval: true`;
  - the always-on sources: every published snippet the published
    `docs/general/index.yaml` selects — its `expanded` ids and the ids of
    every `agents` list — with their transitive required reads.
- **Prose digest.** Over the committed raw content of every evaluated
  snippet and every always-on source, each once.
- **Units digest.** Over the always-on sources: for each, the body of its
  current compressed unit without the unit's frontmatter, or a mark that it
  has none.
- **The fingerprint** is derived from the prose digest and the units digest
  together.
- **Determinism.** The same published snippets, units and
  `docs/general/index.yaml` yield the same two digests and the same
  fingerprint, whichever project's sync computed them, in whatever order the
  projects published, and whichever agent runtimes the computer has
  configured.
- **Change.** A change of the raw content of an evaluated snippet or an
  always-on source, a planned block included, changes the prose digest and
  so the fingerprint, whatever its unit holds. A change of an always-on
  source's current unit body, and a unit becoming current or ceasing to be,
  changes the units digest and so the fingerprint while the prose digest
  stays as it was. Nothing else changes either digest.
- **Empty.** A corpus with no evaluated snippet and no always-on source has a
  defined fingerprint of its own, distinct from any non-empty one.
- **Association.** State records the fingerprint and its two digests per
  published commit, rather than only overwriting a current digest. The
  current fingerprint remains available to work; no separate eval log is
  maintained.
- **When.** Computed by every sync that publishes, as part of the same
  publication, from the corpus as it stands then. A record is written, with
  the sync's time, only when what it holds differs from what was computed:
  the per-commit record when its fingerprint, either digest or its snippet
  count differs, the current record when its fingerprint or either digest
  differs. A record whose content is unchanged is left as it is, its time
  included. That a commit was published before never decides it: another
  project may have changed evaluated doctrine since, which changes the
  fingerprint of an unchanged commit.

## Use cases

The sync scenarios in
`project/instrukt/spec/feature/publication/project-publication` exercise
fingerprint computation as part of publication.

## Known caveats

- Only what the home publishes from `docs/general/` and `docs/domain/<x>/`, and the units of always-on sources, enter the fingerprint; no `docs/project/` snippet, project index or unit a project's own `AGENTS.md` holds does.
- Both digests hash committed raw content — snippets, and unit bodies without their frontmatter — before read-time expansion.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/feature/publication/project-publication](project-publication.md) — the sync that computes it.
- [project/instrukt/spec/feature/publication/compressed-units](compressed-units.md) — the units it covers.
