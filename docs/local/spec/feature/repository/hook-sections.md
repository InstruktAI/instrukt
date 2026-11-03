---
description: instrukt's git hook sections — one marked section per hook file beside whatever else is there, never `exec`ing a command, never blocking git; post-hooks start publication, pushes and pulls advancing main are handed to blueprint, `core.hooksPath` is honoured, and status reports a section a hook manager dropped.
delivered_by: [instrukt-first-build]
---
# Hook Sections — Spec

## What it is

Git hooks are shared: a project, its hook manager and other tools all put
behaviour there. instrukt owns one marked section per hook file it uses and
touches nothing else. Its post-hook sections keep publication current after
every change to main — commit, merge, rebase or checkout — by starting a sync
and returning at once. When blueprint is enabled, its hook sections hand
pushes to main and pulls advancing main to blueprint
(`local/projects/instrukt/spec/feature/blueprint/graph-generation`). instrukt adds no lint or test hooks;
projects keep their own tooling.

## Canonical fields

### Hooks used

| Hook | When instrukt installs it | What the hook run does |
| --- | --- | --- |
| `post-commit` | always | starts `instrukt sync` detached |
| `post-merge` | always | starts `instrukt sync` detached; when a pull advances main and blueprint is enabled, hands the revision to blueprint |
| `post-rewrite` | always | starts `instrukt sync` detached; when a pull through rebase advances main and blueprint is enabled, hands the revision to blueprint |
| `post-checkout` | always | starts `instrukt sync` detached |
| `pre-push` | the blueprint module is on | hands a push to main to the blueprint, which starts generation detached (`local/projects/instrukt/spec/domain/blueprint`) |

- Graph-generation policy belongs to
  `local/projects/instrukt/spec/feature/blueprint/graph-generation`; each
  handoff starts generation detached. Local commits and unrelated merges
  are not graph-generation triggers.
- A started sync runs under the publication lock and does nothing when main
  equals the published commit
  (`local/projects/instrukt/spec/feature/publication/project-publication`), so
  hooks firing on a branch, in a linked worktree or several times in a row
  cost nothing.
- A hook run returns without waiting for what it started and never fails the
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

### Use cases

#### UC-HK1: A developer adds instrukt's hooks without losing their own

```gherkin
Given a developer's repository with its own post-commit hook
When the developer installs instrukt's hooks
And then commits
Then the developer's own hook runs as before
And the commit starts publication of the project
```

#### UC-HK2: A developer's pull keeps their agents' docs current without waiting

```gherkin
Given a developer's repository with instrukt's hooks installed
When the developer pulls new commits to main, by merge or by rebase
Then the pull completes without waiting for publication
And publication of the new main is started
```

#### UC-HK3: A developer whose hook manager dropped instrukt's section restores it

```gherkin
Given a developer's repository whose hook files are generated by a hook manager
And the hook manager regenerated a hook file without instrukt's section
When the developer asks for the status of instrukt's hooks
Then they see which hook lost instrukt's section
And they see the line to add to their hook manager to restore it
```

#### UC-HK4: A developer removes instrukt's hooks while keeping their own

```gherkin
Given a developer's repository with instrukt's hooks installed beside their own hook content
When the developer removes instrukt's hooks
Then instrukt's sections are absent
And the developer's own hook content and hook files remain unchanged
```

## Known caveats

- An existing non-executable hook is left non-executable and install reports it.
- The pre-push section is installed only while blueprint is on; `hooks install` removes it when blueprint is switched off.

## See Also

- docs/local/spec/domain/repository.md — the repository context.
- docs/local/spec/feature/publication/project-publication.md — what a started sync does.
