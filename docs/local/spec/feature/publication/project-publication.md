---
description: A project publishes only its own part, from the committed state of its canonical main — domains, bundles and index entries patched into the global index by owner, with duplicate-id and domain-ownership refusals, retraction of what it no longer owns, and a no-op when main equals the published commit.
delivered_by: [instrukt-first-build]
---
# Project Publication — Spec

## What it is

`instrukt sync` makes a project's committed documentation what agents on this
computer read. It publishes the project the caller stands in, from the
committed state of the project's canonical main, into the home. Each project
publishes only its own part: the global index is patched by owner, so two
projects publishing in turn never take each other's entries, and no command
reads another project's repository. Unless `--force` is supplied, a sync whose main equals the commit it
last published does nothing, so it is cheap to trigger as often as git
changes main (`local/projects/instrukt/spec/feature/repository/hook-sections`).

## Canonical fields

`instrukt sync --force` republishes when main is unchanged. Validation findings refuse publication and preserve the previous publication.

### Source

- **Which project.** The project whose working tree the caller stands in,
  identified by the `project` slug in its `instrukt.yaml`. A directory with no
  project layer, or a project layer with no slug, is refused.
- **Which state.** The committed tree of canonical main: the default branch as
  it stands in the project's primary worktree. Invoked from a linked worktree,
  sync publishes the same canonical main. Uncommitted and unmerged changes are
  never published.
- **Configuration first.** Before publishing, sync brings both configuration
  layers to the current schema version
  (`local/projects/instrukt/spec/domain/configuration`).

### The no-op

- Every successful sync records the commit it published beside the project's
  checkout path under `~/.instrukt/state/publication/`. The runner consumes
  this existing state (`local/projects/instrukt/spec/domain/runner`).
- Unless `--force` is supplied, when canonical main equals the published commit, sync changes nothing in the home or the project, except removing leftovers a previous sync reported, and exits zero.
- A sync that is refused or fails records no commit, so the next trigger
  tries again.
- Concurrent syncs never interleave: every write to the home's publication
  runs under the docs lock. A sync waiting for the lock compares main with the
  published commit once it holds it.

### What a sync writes

| Output | Where | Content |
| --- | --- | --- |
| Published domains | `~/.instrukt/docs/<domain>/` | each domain in the project's `publishes` setting: its snippets from `docs/global/<domain>/`, published raw |
| Workflow flavor | `~/.instrukt/docs/<domain>/workflow/` | the owning project’s `docs/global/<domain>/workflow/` definitions, actions and templates, copied with the published domain and retracted by the same owner rules; not snippet index entries |
| Publisher record | `~/.instrukt/docs/<domain>/publisher.yaml` | the owning project and the domain's trigger line |
| Bundle | `~/.instrukt/docs/<domain>/bundle.md` | the domain's baseline manifest compiled, then its progressive baseline index and activatable procedures |
| Global entries | the global index | one entry per published snippet, owned by the project |
| Local entries | the global index | one entry per snippet under the project's `docs/local/`, owned by the project, carrying the absolute path of the file in the project's primary worktree |
| `AGENTS.md` files | the project's primary worktree | `local/projects/instrukt/spec/feature/publication/agent-entrypoints` |
| Global agent entrypoints | the agent CLIs' user directories | only from the `general` owner (`local/projects/instrukt/spec/feature/publication/agent-entrypoints`) |
| Publication state | `~/.instrukt/state/publication/` | the project's published commit and checkout path |
| Eval fingerprint | `~/.instrukt/state/eval-fingerprint/` | digest associated with the published commit; `local/projects/instrukt/spec/feature/publication/eval-fingerprint` |

- Snippets are published raw: their `@exec` directives run when they are read
  (`local/projects/instrukt/spec/domain/docs`). Compiled outputs — bundles,
  `AGENTS.md` files and global agent entrypoints — expand `@exec` at publish
  time, run in the project root with a 10-second limit, with the runtime
  mapping's session variables removed from the environment, so no compiled
  output captures the session that happened to publish it. A directive that
  fails or times out fails the sync.
- Compilation never inlines a pending snippet and drops planned material, so
  compiled outputs carry the current contract only.
- Local snippet content stays in the project; only its index entries enter
  the home.
- Pending snippets and their companions are published and indexed with their
  pending ownership; retrieval decides what a reader sees
  (`local/projects/instrukt/spec/domain/docs`).

### Patching the global index

Under the docs lock, sync reads the global index, drops every entry the
project owns, checks the project's new entries against the entries that
remain, adds them, writes the result to a temporary file and renames it into
place. Entries of other projects are carried over unchanged.

| Refusal | Condition | Outcome |
| --- | --- | --- |
| Duplicate id | a new entry's id is already held by another project's entry | sync refused, naming the id and its owner |
| Domain owned by another project | a domain the project publishes has a publisher record naming another project, or carries another project's global entries | sync refused, naming the domain and its owner |

A refused sync changes nothing in the home or the project, records no
commit, and exits non-zero. A domain becomes free to claim once its owner's
sync retracts it.

### Retraction

Everything the project published before and its committed main no longer
yields is removed by the same sync: a domain it stopped publishing (its
directory, publisher record and bundle), snippets deleted or moved within a
published domain, its index entries without a source, and an `AGENTS.md`
whose master no longer exists. Retraction touches nothing another project
owns.

### Failure

A sync that fails after it started writing leaves the project's previous
publication readable and records no commit. Compilation that cannot resolve a
required read, or leaves an `@` reference unexpanded, fails the sync and
names the file and the reference.

A path the project's repository controls (a compiled output's location, or a
directory publication writes into) that is a symbolic link refuses the sync
before anything is written; the link's target, the previous publication and
the published commit are untouched. Files in the user's own agent-CLI
directories are written through the user's links, because the repository
cannot create those.

Once the new publication is in place and its commit recorded, failing to
remove the replaced publication's leftover copy does not fail the sync: the
sync reports the leftover path and succeeds, and a later sync removes it.

A refused or failed sync changes nothing in the home or the project except removing leftovers a previous sync reported.

Listing and reading the published corpus and index belong to
`local/projects/instrukt/spec/feature/docs/progressive-retrieval`.

### Use cases

#### UC-PUB1: A developer publishes committed docs on their computer

```gherkin
Given a developer's project that publishes a domain
And the developer commits a new snippet to the project's main
When publication runs for the project
Then the published corpus contains the snippet as committed and the index contains its entry
And uncommitted changes are absent from the published corpus
```

#### UC-PUB2: Two projects publish on one computer without taking each other's docs

```gherkin
Given a developer with two projects on one computer, each publishing its own domain
When the developer publishes each project in turn
Then the documentation index holds the entries of both projects
And each project's publication changed only its own entries
When the developer tries to publish from the second project a domain the first project owns
Then its publication is refused and names the domain and its owner
And the documentation of both projects is as it was before
```

#### UC-PUB3: A developer retracts retired documentation

```gherkin
Given a developer's project whose published documentation includes a snippet and a domain
And a directory of the project holds agent instructions compiled from a source in that directory
When the developer deletes the snippet, stops publishing the domain, deletes that instruction source and commits to main
And publication runs for the project
Then the published corpus and index no longer contain the retired snippet or domain
And the compiled agent instructions of the deleted source are gone
And instructions compiled from sources that still exist are unchanged
And another project's published documentation is unchanged
```

#### UC-PUB4: Publishing again changes nothing until something new is committed

```gherkin
Given a developer's project whose committed main is already published
When publication runs for the project again
Then nothing in the published corpus, the index or the project changes
When the developer asks for the project to be republished regardless
Then its documentation is published again from its committed main
```

#### UC-PUB5: A developer who commits invalid docs keeps the last good publication

```gherkin
Given a developer's project whose documentation is published
When the developer commits documentation that fails validation
And publication runs for the project
Then publication is refused and names each finding
And the previously published documentation and index are unchanged
And the next publication of the project tries that commit again
```

#### UC-PUB6: Compilation that cannot complete stops publication

```gherkin
Given a developer's project whose documentation is published
When its compiled agent instructions run a command at publication and that command fails
And publication runs for the project
Then publication fails and names the file and the command
And the previously published documentation is unchanged
When its documentation passes validation but a compiled bundle would keep an unresolved reference
And publication runs for the project
Then publication fails and names the bundle and the reference
And the previously published documentation is unchanged
```

#### UC-PUB7: A committed link cannot make publication write outside the project

```gherkin
Given a developer's project whose documentation is published
When a commit replaces a location publication writes with a link to a file elsewhere
And publication runs for the project
Then publication is refused and names that location
And the file the link points to is unchanged
And the previously published documentation is unchanged
```

#### UC-PUB8: A leftover that cannot be removed does not undo a publication

```gherkin
Given a developer's project with new documentation committed to main
And the copy it replaces cannot be removed
When publication runs for the project
Then the new documentation is published and the developer is told which leftover remains
When the leftover can be removed again and publication runs
Then the leftover is gone
```

## Known caveats

- Local index entries point at the file in the project's primary worktree,
  while the entries themselves come from committed main; a local snippet edited
  but not committed is read as edited.
- The default branch is the remote's default (`origin/HEAD`); without a remote, it is the local repository's current main line as git reports its default.
- Include fragments and third-party docs are published by their owning project like its domains, with owner-scoped patching.
- A sync refuses committed docs with structural validation findings, reports the findings, and retains the previous publication.

## See Also

- docs/local/spec/domain/publication.md — the publication context.
- docs/local/spec/domain/docs.md — what is read from the home and how.
