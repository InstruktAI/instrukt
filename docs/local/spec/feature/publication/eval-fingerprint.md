---
description: "The eval fingerprint — a digest of every published snippet that declares `eval: true`, recomputed by every sync into the home's state, so that every stamp can name the doctrine it was produced under."
delivered_by: [instrukt-first-build]
---
# Eval Fingerprint — Spec

## What it is

A snippet that declares `eval: true` in its frontmatter is part of the
doctrine whose effect is evaluated. The eval fingerprint identifies the
doctrine in force on this computer: a digest over those snippets. Every sync
that publishes recomputes it into `~/.instrukt/state/eval-fingerprint/`,
recorded per published commit. The current fingerprint is available to work;
stamp copying and reporting belong to
`local/projects/instrukt/spec/domain/work`. Eval has no module and no switch; the fingerprint
is always computed.

## Canonical fields

### Shipped process set

The flavor marks prepare, build, review and quality-lane procedures and policies
with `eval: true`. Shared lifecycle, orchestration, commit and documentation
rules serving those phases are included. Coordinator, integrator, monitoring
and the report procedure itself are outside this shipped process set.

### The fingerprint

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
- **When.** Written by every sync that publishes, as part of the same
  publication. A sync that does nothing (main equals the published commit)
  leaves it as it is.

## Known caveats

- Only published snippets (from `docs/general/` and `docs/domain/<x>/`) with `eval: true` enter the fingerprint; `docs/project/` snippets do not.
- The digest hashes committed raw snippet content, before read-time expansion.

## See Also

- docs/local/spec/domain/publication.md — the publication context.
- docs/local/spec/feature/publication/project-publication.md — the sync that computes it.
