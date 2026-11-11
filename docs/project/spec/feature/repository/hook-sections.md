---
description: instrukt's git hook sections — one marked section per hook file beside whatever else is there, never `exec`ing a command; checkout renders before return and other post-hooks start publication detached, pushes and pulls advancing main are handed to blueprint, `core.hooksPath` is honoured, and status reports a section a hook manager dropped.
---
# Hook Sections — Spec

## What it is

Git hooks are shared: a project, its hook manager and other tools all put
behaviour there. instrukt owns one marked section per hook file it uses and
touches nothing else. Its post-hook sections keep publication current after
every change to main — commit, merge or rebase — by starting a sync
and returning at once. When blueprint is enabled, its hook sections hand
pushes to main and pulls advancing main to blueprint
(`project/instrukt/spec/feature/blueprint/graph-generation`). instrukt adds no lint or test hooks;
projects keep their own tooling.

## Canonical fields

### Hooks used

| Hook | When instrukt installs it | What the hook run does |
| --- | --- | --- |
| `pre-commit` | always | converts an older project layer to the current schema and inserts or corrects the editor modelines of the files being committed, staging both into the commit being made (`project/instrukt/spec/feature/configuration/owned-files`, `project/instrukt/spec/feature/publication/editor-schemas`) |
| `post-commit` | always | starts `instrukt sync` detached; after a partial commit, returns an index entry the pre-commit conversion left in its unconverted form to the committed version, leaving every other staging choice as it is |
| `post-merge` | always | starts `instrukt sync` detached; when a pull or runner fast-forward advances main to its fetched upstream head and blueprint is enabled, hands the revision to blueprint |
| `post-rewrite` | always | starts `instrukt sync` detached; when a pull through rebase advances main and blueprint is enabled, hands the revision to blueprint |
| `post-checkout` | always | renders project entrypoints in this checkout from published state; an unpublished primary checkout bootstraps full publication from main once; a previously published primary checkout on main also retains its detached sync trigger |
| `pre-push` | the blueprint module is on | hands a push to main to the blueprint, which starts generation detached (`project/instrukt/spec/domain/blueprint`) |

- Graph-generation policy belongs to
  `project/instrukt/spec/feature/blueprint/graph-generation`; each
  handoff starts generation detached. Local commits and unrelated merges
  are not graph-generation triggers.
- Checkout rendering is the synchronous entrypoint-only operation in
  `project/instrukt/spec/feature/publication/agent-entrypoints`, including its
  primary-checkout bootstrap and missing-publication behavior. It runs on every
  branch and in linked worktrees; other post-hooks retain their main guard.
- A started sync runs under the publication lock and does nothing when main
  equals the published commit
  (`project/instrukt/spec/feature/publication/project-publication`), so
  repeated publication triggers do not republish an unchanged revision.
  Checkout rendering follows its separate synchronous contract above.
- Publication and graph generation started by other hooks stay detached.
  Checkout rendering completes before return; no hook ever fails the
  git operation: a missing `instrukt` binary, a refused sync or a failed start
  leaves git's result unchanged.

### The section

- Delimited by `# >>> instrukt >>>` and `# <<< instrukt <<<`, holding the one
  call `instrukt hooks run <hook>`.
- Never `exec`s a command (never replaces the shell) and never ends the hook,
  so every section after it runs; a redirection-only `exec` is allowed.
- **Pre-push stdin.** Git passes pre-push its ref list on stdin, which every
  section of the hook may need. The pre-push section copies stdin to a
  temporary file, runs `instrukt hooks run pre-push` with that file on its
  stdin, then restores the hook's stdin from the file (`exec 0<"$tmp"`) and
  removes the file. The open descriptor keeps the list readable, so every
  section after it reads the full ref list and nothing stays behind in the
  temporary directory.

### `instrukt hooks install`

- Writes the section into each hook file of the table above, in the hooks
  directory: the directory `core.hooksPath` names when it is set — the
  setting itself is never changed — otherwise the repository's own.
- **No file:** creates it, executable, with an interpreter line and the
  section.
- **A file without the section:** appends the section, leaving every existing
  line as it was. When the file contains a line that runs `exec`, the section
  is inserted before the first such line instead, and install reports that it
  did so.
- **A file with the section:** replaces the section's content when it
  differs; otherwise changes nothing. Reinstalling is idempotent.
- Never overwrites a file and never touches a line outside its section.
- Reports per hook whether the section was written, updated or already
  current.

### `instrukt hooks remove`

- Removes the section from each hook file that holds one; every other line,
  and the file itself, remain.

### `instrukt hooks status`

- Per hook of the table above: the hooks directory in use (and that
  `core.hooksPath` chose it), and whether the section is current, outdated or
  absent.
- Reports each section's last run outcome, including a refused sync.
- For an absent section, prints the one line to add to the tool that manages
  that hook file — the hook-run call — so a hook manager that regenerates the
  file keeps instrukt's behaviour.

## Use cases

### UC-HK1: A developer adds instrukt's hooks without losing their own

```gherkin
Given a developer's repository with its own post-commit hook
When the developer installs instrukt's hooks
And then commits
Then the developer's own hook runs as before
And the commit starts publication of the project
```

### UC-HK2: A developer's pull keeps their agents' docs current without waiting

```gherkin
Given a developer's repository with instrukt's hooks installed
When the developer pulls new commits to main, by merge or by rebase
Then the pull completes without waiting for publication
And publication of the new main is started
```

### UC-HK3: A developer whose hook manager dropped instrukt's section restores it

```gherkin
Given a developer's repository whose hook files are generated by a hook manager
And the hook manager regenerated a hook file without instrukt's section
When the developer asks for the status of instrukt's hooks
Then they see which hook lost instrukt's section
And they see the line to add to their hook manager to restore it
```

### UC-HK4: A developer removes instrukt's hooks while keeping their own

```gherkin
Given a developer's repository with instrukt's hooks installed beside their own hook content
When the developer removes instrukt's hooks
Then instrukt's sections are absent
And the developer's own hook content and hook files remain unchanged
```

### UC-HK5: A developer keeps their checkout hooks working through the rendering handoff

```gherkin
Given a developer has installed instrukt post-checkout beside their own hook content
When the developer checks out a branch or adds a linked worktree
Then the hook waits for the rendering handoff to finish before returning to git
And the developer's own hook still runs
And a rendering failure leaves git's result unchanged and is reported
```

## Known caveats

- An existing non-executable hook is left non-executable and install reports it.
- The pre-push section is installed only while blueprint is on; `hooks install` removes it when blueprint is switched off.

## See Also

- [project/instrukt/spec/domain/repository](../../domain/repository.md) — the repository context.
- [project/instrukt/spec/feature/publication/project-publication](../publication/project-publication.md) — what a started sync does.
