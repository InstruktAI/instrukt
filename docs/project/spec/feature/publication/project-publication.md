---
description: A project publishes only its own general, domain and project positions from committed canonical main, claimed by owner, index entries patched into the global index by owner, with duplicate-id, ownership and reserved-name refusals, retraction of what it no longer owns, and a no-op when main equals the published commit.
delivered_by: [instrukt-first-build]
---
# Project Publication — Spec

## What it is

`instrukt sync` makes a project's committed documentation what agents on this
computer read. It publishes the project the caller stands in, from the
committed state of the project's canonical main, into the home. Each project
publishes only its own part: every position it publishes is claimed by it
alone, the global index is patched by owner, so two projects publishing in
turn never take each other's entries, and no command reads another project's
repository. Unless `--force` is supplied, a sync whose main equals the commit
it last published does nothing, so it is cheap to trigger as often as git
changes main (`project/instrukt/spec/feature/repository/hook-sections`).

## Canonical fields

`instrukt sync --force` republishes when main is unchanged. Validation findings
refuse publication and preserve the previous publication.

### Source

- **Which project.** The project whose working tree the caller stands in,
  identified by the `project` slug in its `instrukt.yaml`. A directory with no
  project layer, or a project layer with no slug, is refused.
- **Which state.** The committed tree of canonical main: the default branch as
  it stands in the project's primary worktree. Invoked from a linked worktree,
  sync publishes the same canonical main. Uncommitted and unmerged changes are
  never published.
- **Which positions.** What the committed tree holds decides what the project
  publishes: `docs/general/` when it exists, and each directory
  `docs/domain/<x>/`, published as domain `<x>`. No setting declares them.
  `docs/project/` is published under `~/.instrukt/docs/project/<slug>/…`,
  where its ids remain `project/<slug>/…`.
- **Configuration first.** Before publishing, sync brings both configuration
  layers to the current schema version before any refusal or
  unchanged-publication skip
  (`project/instrukt/spec/domain/configuration`).

### The no-op

- Every successful sync records the commit it published beside the project's
  checkout path under `~/.instrukt/state/publication/`. The runner consumes
  this existing state (`project/instrukt/spec/domain/runner`).
- After configuration migration, unless `--force` is supplied, when canonical
  main equals the published commit, sync changes nothing in the home or the
  project, except removing
  leftovers a previous sync reported, and exits zero.
- A sync that is refused or fails records no commit, so the next trigger
  tries again.
- Concurrent syncs never interleave: every write to the home's publication
  runs under the docs lock. A sync waiting for the lock compares main with the
  published commit once it holds it.

### What a sync writes

| Output | Where | Content |
| --- | --- | --- |
| `general` | `~/.instrukt/docs/general/` | the snippets of `docs/general/`, published raw, with its `index.yaml` and `schema.yaml` when present |
| Domains | `~/.instrukt/docs/<x>/` | for each `docs/domain/<x>/`: its snippets, published raw, with its `index.yaml` and `schema.yaml` when present |
| Workflow flavor | `~/.instrukt/docs/<x>/workflow/` | the owning project's `docs/domain/<x>/workflow/` definitions, actions and templates, copied with the domain and retracted by the same owner rules; not snippet index entries |
| Claims | the ownership record under `~/.instrukt/state/publication/` | the project as owner of each position it publishes |
| Published entries | the global index `~/.instrukt/state/index.yaml` | one entry per published snippet, owned by the project |
| Project position and entries | `~/.instrukt/docs/project/<slug>/…` and the global index | committed `docs/project/` content, including pending companions and its position files; entries owned by that project, ids `project/<slug>/…`, resolving to published files with source checkout provenance |
| `AGENTS.md` files | the project's primary worktree | `project/instrukt/spec/feature/publication/agent-entrypoints` |
| Global agent files | the agent CLIs' user directories | only from the owner of `docs/general/` (`project/instrukt/spec/feature/publication/agent-entrypoints`) |
| Editor schemas and modelines | the project's primary worktree | `project/instrukt/spec/feature/publication/editor-schemas` |
| Publication state | `~/.instrukt/state/publication/` | the project's published commit and checkout path, and the written record of every user-scope instruction file it wrote |
| Eval fingerprint | `~/.instrukt/state/eval-fingerprint/` | digest associated with the published commit; `project/instrukt/spec/feature/publication/eval-fingerprint` |

- Snippets are published raw: their `@config` directives resolve when they are
  read (`project/instrukt/spec/domain/docs`). Rendered outputs —
  `AGENTS.md` files and global agent files — resolve `@config` at
  publication, in-process against the publishing project's effective
  configuration with no runtime session, so no rendered output captures a
  session. A `runtime.*` path in a rendered output is a validation finding,
  since publication has no session to resolve it against; any other path that
  cannot be resolved fails the sync. Publication runs no command: an `@exec`
  directive in a published snippet or master is a validation finding, and
  sync refuses and names the file.
- Rendering never inlines a pending snippet and drops planned material, so
  rendered outputs carry the current contract only.
- Project snippets are copied from committed main into the project store;
  retrieval of that copy belongs to
  `project/instrukt/spec/feature/docs/progressive-retrieval`.
- Pending snippets and their companions are published and indexed with their
  pending ownership; retrieval decides what a reader sees
  (`project/instrukt/spec/domain/docs`).

### Subscription entries and index identity

The same atomic owner-scoped index patch carries a delivered procedure's validated
patterns, home kind, identity, attendance, delivery settings, effective role,
required-read references and position reach. An omitted subscribed role resolves
to `event-handler`; explicit roles remain independent of workflow role lists.
Role class and scoped grants are read from the machine's `agents.yaml`, not
copied into a second publication role catalog. Pending entries retain their
pending marker and are excluded by activation. Project-local subscription reach is preserved for
the owning project; general/domain entries reach all origins of their declared
kind. Index rendering lists subscribed procedures as configured responsibilities
with patterns, never self-service triggered procedures. Events-off leaves those
entries present and inert.

Removing or editing a subscription changes the index identity at publication;
the next routed event reloads it without a separate trigger. Invalid declaration
refuses publication and keeps the last good index. A runtime read failure clears
the active subscription set rather than keeping stale entries. Publication
registers no runtime session and executes no event reaction.

### Claims and patching

Under the docs lock, sync checks the project's claims against the ownership
record, reads the global index, drops every entry the project owns, checks
the project's new entries against the entries that remain, adds them, writes
the result to a temporary file and renames it into place. Entries and claims
of other projects are carried over unchanged.

| Refusal | Condition | Outcome |
| --- | --- | --- |
| Reserved name | a directory `docs/domain/general/`, `docs/domain/project/` or `docs/domain/third-party/` | sync refused, naming the directory and the rule that `general` and `project` are reserved, `third-party` keeps its special case, and no domain may take them |
| Position owned by another project | `docs/general/`, or a `docs/domain/<x>/`, already claimed on this computer by another project | sync refused, naming the position and its current owner |
| Duplicate id | a new entry's id is already held by another project's entry | sync refused, naming the id and its owner |

After configuration migration, a refused sync changes no publication output,
records no published commit, and exits non-zero. A position becomes free to
claim once its owner's
sync retracts it. No name is reserved beyond `general` and `project`, besides
`third-party`'s existing special case, and no claim is curated.

### What publication renders and refuses by

- The `index.yaml` of each position — its allowed keys, what `expanded`,
  `listed` and `agents` select and their findings — is the docs context's
  model (`project/instrukt/spec/domain/docs`). A finding in a
  published position's `index.yaml`, or in `docs/project/index.yaml`, refuses
  the sync like any validation finding.
- A domain without `index.yaml` is published and indexed, and the sync
  reports a notice naming it: it is absent from the global agent files and
  found only through `instrukt docs index --domain <x>`. The notice never
  refuses the sync.

### Retraction

Everything the project published before and its committed main no longer
yields is removed by the same sync: a position it stopped publishing (its
directory under `~/.instrukt/docs/`, its claim, its entries), snippets deleted
or moved within a published position, its index entries without a source, an
`AGENTS.md` whose master no longer exists, and, when it stops publishing
`docs/general/`, the global agent files it wrote. Retraction touches nothing
another project owns.

### Failure

A sync that fails after it started writing leaves the project's previous
publication readable and records no commit. Rendering that cannot resolve a
required read, or leaves a reference unresolved, fails the sync and names the
file and the reference.

A path the project's repository controls (a rendered output's location, or a
directory publication writes into) that is a symbolic link refuses the sync
before anything is written; the link's target, the previous publication and
the published commit are untouched. Files in the user's own agent-CLI
directories are written through the user's links, because the repository
cannot create those.

Once the new publication is in place and its commit recorded, failing to
remove the replaced publication's leftover copy does not fail the sync: the
sync reports the leftover path and succeeds, and a later sync removes it.

Configuration migration precedes publication admission. After that migration,
a refused or failed sync changes no publication output in the home or project
except removing leftovers a previous sync reported. A refused or failed sync still
reports the leftovers it could not remove.

Listing and reading the published corpus and index belong to
`project/instrukt/spec/feature/docs/progressive-retrieval`.

## Use cases

### UC-PUB1: A developer publishes committed docs on their computer

```gherkin
Given a developer's project with a domain of its own
And the developer commits a new snippet to that domain on the project's main
When publication runs for the project
Then the published corpus contains the snippet as committed and the index contains its entry
And uncommitted changes are absent from the published corpus
```

### UC-PUB2: Two projects publish on one computer without taking each other's docs

```gherkin
Given a developer with two projects on one computer, each with a domain of its own
When the developer publishes each project in turn
Then the documentation index holds the entries of both projects
And each project's publication changed only its own entries
When the developer adds to the second project a domain the first project owns and publishes it
Then its publication is refused and names the domain and its owner
And the documentation of both projects is as it was before
```

### UC-PUB3: A developer retracts retired documentation

```gherkin
Given a developer's project whose published documentation includes a snippet and a domain
And a directory of the project holds agent instructions rendered from a master in that directory
When the developer deletes the snippet, removes the domain, deletes that master and commits to main
And publication runs for the project
Then the published corpus and index no longer contain the retired snippet or domain
And the rendered agent instructions of the deleted master are gone
And instructions rendered from masters that still exist are unchanged
And another project's published documentation is unchanged
```

### UC-PUB4: Publishing again changes nothing until something new is committed

```gherkin
Given a developer's project whose committed main is already published
When publication runs for the project again
Then after any required configuration migration, nothing in the published corpus, the index or the project changes
When the developer asks for the project to be republished regardless
Then its documentation is published again from its committed main
```

### UC-PUB5: A developer who commits invalid docs keeps the last good publication

```gherkin
Given a developer's project whose documentation is published
When the developer commits documentation that fails validation
And publication runs for the project
Then publication is refused and names each finding
And the previously published documentation and index are unchanged
And the next publication of the project tries that commit again
```

### UC-PUB6: Rendering that cannot complete stops publication

```gherkin
Given a developer's project whose documentation is published
When its rendered agent instructions ask for a configured value that cannot be resolved at publication
And publication runs for the project
Then publication fails and names the file and the value
And the previously published documentation is unchanged
When its documentation passes validation but a rendered instruction file would keep an unresolved reference
And publication runs for the project
Then publication fails and names the file and the reference
And the previously published documentation is unchanged
```

### UC-PUB7: A committed link cannot make publication write outside the project

```gherkin
Given a developer's project whose documentation is published
When a commit replaces a location publication writes with a link to a file elsewhere
And publication runs for the project
Then publication is refused and names that location
And the file the link points to is unchanged
And the previously published documentation is unchanged
```

### UC-PUB8: A leftover that cannot be removed does not undo a publication

```gherkin
Given a developer's project with new documentation committed to main
And the copy it replaces cannot be removed
When publication runs for the project
Then the new documentation is published and the developer is told which leftover remains
But when publication runs again and is refused for another reason
Then the developer is still told which leftover remains
When the leftover can be removed again and publication runs
Then the leftover is gone
```

### UC-PUB9: A developer who names a domain after a reserved name is told why it is refused

```gherkin
Given a developer's project that commits a domain named general or project
When publication runs for the project
Then publication is refused and names the domain's directory
And it says that general and project are reserved and no domain may take them
And the previously published documentation is unchanged
```

## Known caveats

- Uncommitted or unmerged project edits remain outside the project store;
  a failed sync retains the previous project publication with its index.
- The default branch is the remote's default (`origin/HEAD`); without a
  remote, it is the local repository's current main line as git reports its
  default.
- Third-party docs are published by their owning project like its positions,
  with owner-scoped patching.
- A sync refuses committed docs with structural validation findings, reports
  the findings, and retains the previous publication.

## See Also

- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- [project/instrukt/spec/domain/docs](../../domain/docs.md) — the `index.yaml` model and what is read from the home and how.
- [project/instrukt/spec/feature/publication/editor-schemas](editor-schemas.md) — the editor schemas and modelines a sync refreshes.
