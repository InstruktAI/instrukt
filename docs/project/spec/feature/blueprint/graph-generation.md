---
description: A graph per push- or pull-obtained revision of main — started detached when the push is seen so the push never waits, one generation at a time per project with a newer revision superseding a queued one, stored under its SHA in the home, and never replaced by an interrupted run.
delivered_by: [instrukt-first-build]
---
# Graph Generation — Spec

## What it is

The project's map follows remote-main revisions obtained by push or pull, not every local commit.
When a push to main is seen, the graph of the pushed revision is generated in
the background without making the push wait. Each graph is kept under its
revision's SHA; agents read the latest complete one. Generation runs on each
developer's machine from code it already has, neither in CI nor after CI.

## Canonical fields

A pull moving remote main also starts generation through post-merge/post-rewrite. Graphs are never generated per local commit. Keep only the latest complete graph and its predecessor.

### Trigger

- “Main” means the repository's default branch, whatever its name.
- The repository context's pre-push section
  (`project/instrukt/spec/domain/repository`) sees a push whose
  destination is `main`, records the SHA being pushed and starts generation
  detached; it returns at once. Pushes to other branches start nothing.
- The section is installed only while the blueprint module is on. It never
  fails the push: a generation that cannot start or fails leaves the push
  unaffected.
- Local commits and checkouts that do not advance main start no generation. A pull advancing main starts detached generation through post-merge/post-rewrite.

### Generation

- The engine maps the committed tree of the recorded SHA, never a working tree
  (`project/instrukt/spec/feature/blueprint/graph-engine`).
- A SHA whose complete graph already exists is not generated again.
- **One at a time per project.** While a generation runs, a newly recorded
  SHA waits as the queued revision; a newer SHA replaces a queued older one.
  The running generation finishes; then the queued revision, if any, starts.
- **Complete or nothing.** A graph is written to a staging place, validated
  against the contract, and only then made the project's latest graph in one
  step. A generation that fails, is refused or is interrupted leaves no partial
  graph visible and leaves the previous latest graph in place.

### Generation status

- `instrukt blueprint status` shows the last generation's outcome; `affected` names a failed generation newer than the graph it answers from.

### Storage

- `~/.instrukt/state/blueprint/<project>/<sha>/` holds one revision's graph;
  `<project>` is the slug in the project layer of `instrukt.yaml`.
- The latest graph is the complete graph of the most recently obtained remote-main revision
  that has one. Every worktree of the project reads the same store.
- The store is a cache: deleting it loses nothing the next push cannot
  re-create.

### Use cases

#### UC-BG1: A developer pushes to main without waiting and the revision gets mapped

```gherkin
Given a project with the blueprint module on
When a developer pushes a revision to main
Then the push completes without waiting for the map
And once mapping finishes, agents' structural answers reflect that revision and name it
When several revisions are pushed while a mapping is still running
Then the newest of them is mapped next and the ones between are skipped
And agents' answers name the newest mapped revision
```

Extension of UC-BG1: if mapping fails or is interrupted, agents continue
receiving answers from the last complete map, naming that map's revision.

## Known caveats

- Generation is keyed by SHA even if the remote refuses the push; it is harmless and can supply the latest graph. Answers always name their SHA.

## See Also

- docs/project/spec/domain/blueprint.md — the blueprint context.
- docs/project/spec/domain/repository.md — the pre-push section.
