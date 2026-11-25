---
description: "The eval fingerprint — a digest of every published snippet that declares `eval: true` and of the always-on doctrine as the global agent files hold it, compressed units included, recomputed by every sync into the home's state with prose and compression recorded apart, so that every stamp can name the doctrine it was produced under."
---
# Eval Fingerprint — Spec

## What it is

A snippet that declares `eval: true` in its frontmatter is part of the
doctrine whose effect is evaluated. The always-on doctrine every agent starts
with is part of it as well, in the form agents receive it: a source's current
compressed unit where it has one, its prose where it has none
(`project/instrukt/spec/feature/publication/compressed-units`). The eval
fingerprint identifies the doctrine in force on this computer: a digest over
both. Every sync that publishes recomputes it into
`~/.instrukt/state/eval-fingerprint/`, recorded per published commit, with
what tells a change of prose apart from a change of compression alone. The
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

- **Input.** After this sync's patch, from the published corpus of the home:
  - every snippet published from `docs/general/` or a `docs/domain/<x>/` of
    any project whose frontmatter declares `eval: true`;
  - the covered sources: each source the published `docs/general/index.yaml`
    inlines — its `expanded` ids and the ids of every `agents` list, with
    their closures — and each `eval: true` snippet a published project's
    `docs/project/index.yaml` inlines with its closure. A covered source
    enters as the renderings hold it: its current compressed unit's body,
    or, where it has no current unit, its prose.
- **Determinism.** The same snippets, units and published indexes yield the
  same fingerprint, whichever project's sync computed it, in whatever order
  the projects published, and whichever agent runtimes the computer has
  configured.
- **Change.** Adding, removing or changing an `eval: true` snippet, the prose
  of a covered source, or a covered source's current unit changes the
  fingerprint, and so does a covered source's unit becoming current or
  stale; changes to other snippets and units do not.
- **Prose apart from compression.** Beside the fingerprint, state records one
  digest over the prose of every `eval: true` snippet and covered source,
  and one over the covered sources' current units. A change of prose changes
  the first; a change of compression alone changes only the second.
- **Empty.** A corpus with no `eval: true` snippet and no covered source has a
  defined fingerprint of its own, distinct from any non-empty one.
- **Association.** State records the fingerprint and its two digests per
  published commit, rather than only overwriting a current digest. The
  current fingerprint remains available to work; no separate eval log is
  maintained.
- **When.** Computed by every sync that publishes, as part of the same
  publication, from the corpus as it stands then. A record is written, with
  the sync's time, only when what it holds differs from what was computed:
  the per-commit record when its fingerprint, its two digests or its snippet
  count differ, the current record when its fingerprint differs. A record
  whose content is unchanged is left as it is, its time included. That a
  commit was published before never decides it: another project may have
  changed evaluated doctrine since, which changes the fingerprint of an
  unchanged commit.

## Use cases

The sync scenarios in
`project/instrukt/spec/feature/publication/project-publication` exercise
fingerprint computation as part of publication.

## Known caveats

- Only published snippets (from `docs/general/` and `docs/domain/<x>/`) and their units enter the fingerprint; `docs/project/` snippets and their units do not.
- The digest hashes committed raw content — snippets, and the bodies of covered units — before read-time expansion.
- A unit a project's committed `AGENTS.md` holds for a snippet that is not `eval: true` is outside the fingerprint.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/feature/publication/project-publication](project-publication.md) — the sync that computes it.
- [project/instrukt/spec/feature/publication/compressed-units](compressed-units.md) — the units it covers.
