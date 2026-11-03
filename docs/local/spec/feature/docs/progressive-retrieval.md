---
description: Phased documentation retrieval — `instrukt docs index` lists ids, `instrukt docs get` reads snippets through the read pipeline with read-time commands, and required reads are followed explicitly or as one closure.
delivered_by: [instrukt-first-build]
---

# Progressive Retrieval — Spec

## What it is

Agents read documentation in phases so they load only what the task needs:

1. **Index** — `instrukt docs index` lists snippet ids with one-line
   descriptions.
2. **Get** — `instrukt docs get <id…>` returns the selected snippets' bodies
   and names their required reads.
3. **Follow up** — the caller fetches the named required reads with another
   `get`. Required reads are never loaded implicitly, except as a closure
   (`--closure`), which reads one procedure with everything it requires in one
   pass.

Both commands discover snippets through the home (`~/.instrukt`): published
files and index metadata are written there by publication. No index file
exists in a repository. A local snippet's index entry carries the absolute
path of its file in its project, and `get` reads that file from disk, including
another project's local file. Working-tree validation is a separate surface
(`local/projects/instrukt/spec/feature/docs/snippet-validation`). Publication's
own-repository restriction does not restrict these indexed local-file reads.

## Canonical fields

### Index

- **Default view:** every published domain, plus the local snippets of the
  project the caller stands in — the repository containing the working
  directory, matched by the `project` slug its `instrukt.yaml` declares.
  Outside any project the view holds the published domains only.
- **Filters** (each only narrows):

  | Filter | Effect |
  | --- | --- |
  | `--domains <d,…>` | only snippets of these published domains; local snippets and third-party docs drop out; an unknown domain fails non-zero, naming the valid domains |
  | `--areas <t,…>` | only these taxonomy types |
  | `--baseline-only` | only snippets named by a domain's baseline manifest or the project's `docs/local/baseline.md` |
  | `--third-party` | adds the third-party docs section |
  | `--target <slug>` | only snippets the todo delivers or has pending material for (`local/projects/instrukt/spec/feature/docs/pending-documentation`) |
  | `--project <slug>` | lists only that project's local snippets |

- **Entry line:** `<id>: <description>`, followed by `(delivered by: <slugs>)`
  for a pending snippet and `(superseded by: <slugs>)` for a live snippet with
  pending material. Entries are ordered by taxonomy (principle, concept,
  policy, procedure, design, spec, adr), then scope, then id.
- **Shape:** a header naming the available filters, the entries, the
  third-party section when requested, a closing line directing the caller to
  `instrukt docs get` with the ids of interest, and a note explaining the
  pending suffixes and the `--target` view.

### Get

- An id resolves to its file directly: a published id to the file in the home,
  a local id through its index entry to the file in its project. A local id of
  another project is read from any location; no project flag is needed.
- An unknown id prints an error naming it and exits non-zero; known ids in the
  same call are not rendered.
- Each rendered snippet is a header block (id, type, domain, scope,
  description) followed by its body. The output header lists the required
  reads that were not loaded and must have been read at least once.
- Required reads are resolved transitively, by path. A local snippet's required
  reads follow its own project's local snippets and published snippets only.
- A live snippet with pending material, read without `--target`, is followed
  by a notice naming the owning slugs and how to read their view.
- A third-party id renders its body; required-read resolution does not apply.

### Closure

`--closure` takes exactly one id, a delivered procedure, and no `--target`.
A closure request naming several ids, or an id that is not a procedure, is
refused with an error saying a closure is read for exactly one procedure.
It renders the procedure, every transitive required read once, and every
domain bundle those reads name, listing the included ids in its header. For a
procedure carrying root frontmatter (`role`, `subscriptions`,
`activation_trigger` or `dispatch`), a required read that is pending or
resolves to no readable published snippet or bundle refuses the closure,
naming it. Activatable-procedure catalog entries instruct this form.

### Read pipeline

Every body `get` renders passes, in order:

1. **Pending gate** — a pending snippet outside the lens is withheld with a
   notice (`local/projects/instrukt/spec/feature/docs/pending-documentation`).
2. **Companion** — with `--target <slug>`, that slug's companion replaces the
   body when it exists.
3. **`@include`** — each fragment directive is replaced by the fragment's
   body; a fragment that cannot be found leaves its directive visible.
4. **Planned lens** — planned blocks resolved for the lens; hidden content never executes.
5. **`@exec`** — each visible read-time command runs (below).
6. **Inline references** — every `@<path>.md` becomes its absolute path.
7. **Frontmatter** — stripped.
8. **Required reads** — the section is stripped from the body.

### Read-time commands

A read-time command is the directive `<!-- @exec: <command> -->` in a snippet
body. `get` runs it at read time:

- in the caller's working directory and environment, through the shell;
- with a 10-second limit;
- every read, never cached;
- its standard output, when non-empty, replaces the directive as a fenced
  block; empty output replaces it with nothing.

On a non-zero exit or the time limit, the directive is replaced by an inline
notice naming the command and its exit status (or the time limit); `get`
prints the whole output and then exits non-zero.

Published snippets carry the directive unexpanded; only the reader's `get`
runs it. Compiled `AGENTS.md` entrypoints are expanded at publication instead,
which is publication's contract. The configured-value form is
`<!-- @exec: instrukt config get <path> -->`; how the value resolves is
configuration's (`local/projects/instrukt/spec/feature/configuration/interpolation`).

### Publication behind main

When the caller stands in a project whose canonical main is ahead of the
commit publication last published for it, `index` and `get` print a warning
naming both commits alongside their normal output. They publish nothing.

### Use cases

#### UC-DR1: An agent loads the documentation a task needs

```gherkin
Given an agent acting for a developer works in a project that has published its documentation
When it lists the documentation filtered to the areas and domains its task concerns
And it reads the snippets it selects
Then it receives each selected snippet's current text with its references resolved
And it is told which required reads it has not yet loaded
And nothing it did not ask for is loaded
```

#### UC-DR2: An agent starts a procedure with everything it requires

```gherkin
Given an agent acting for a developer is directed to run a published procedure
When it reads that procedure as a closure
Then it receives the procedure, each of its transitive required reads once, and the domain baselines they name, in one read
But when one of those required reads is still pending or cannot be read
Then the read is refused and names the required read that blocked it
But when it asks for a closure of several items at once, or of something that is not a procedure
Then the read is refused and says a closure is read for exactly one procedure
```

#### UC-DR3: An agent reads instructions that carry values from its own session

```gherkin
Given a published procedure whose text asks for a configured value through a read-time command
And two agents acting for the same developer run in different sessions on one machine
When each agent reads that procedure
Then each receives the text with the value computed in its own session at the moment of reading
And the published procedure itself still carries the command, not a value
But when the command fails
Then the agent receives the text with a notice naming the failed command, and the read reports failure
```

## Known caveats

- The planned lens runs before read-time commands, so hidden content never executes.
- `--project <slug>` lists only that project's local docs. An unknown slug lists nothing and says no such project has published.
- A missing local file makes `get` name the project and missing path and direct the caller to republish from that project.

## See Also

- local/projects/instrukt/spec/domain/docs
- local/projects/instrukt/spec/feature/docs/pending-documentation
- local/projects/instrukt/spec/feature/configuration/interpolation
