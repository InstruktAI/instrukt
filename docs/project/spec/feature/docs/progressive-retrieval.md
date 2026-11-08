---
description: Retrieval primitives — `instrukt docs index` lists ids, `instrukt docs get` reads snippets through the read pipeline with configured values, `--closure` reads any snippet with its transitive required reads, and `get --domain` reads a domain on demand.
delivered_by: [instrukt-first-build]
---

# Progressive Retrieval — Spec

## What it is

Agents read documentation in steps so they load only what the task needs, and
the CLI decides nothing on their behalf:

1. **Index** — `instrukt docs index` lists snippet ids with one-line
   descriptions.
2. **Get** — `instrukt docs get <id…>` returns the selected snippets' bodies
   and names their required reads.
3. **Follow up** — the caller fetches the named required reads with another
   `get`. Required reads are never loaded implicitly, except as a closure
   (`--closure`), which reads one snippet with everything it requires in one
   pass.
4. **Domain** — `instrukt docs get --domain <x>` reads what a domain renders
   in full, reached from that domain's one line in the agent's global file.

References are snippet ids everywhere a snippet names another — in
`## Required reads`, See Also and prose — and these commands resolve them; no
snippet names another by path alone. In Required reads and See Also, a
same-repository id may carry a relative Markdown link to its file; the id is
still the reference. References to another repository and bare-domain reads
are plain ids or domain names.

The commands discover snippets through the global index in the home's state
(`~/.instrukt/state/`), written by publication with the published files. No
index file is generated into a repository. Every project snippet is read from
`~/.instrukt/docs/project/<slug>/…`, published from its committed main like
general and domain content. Source checkout paths remain authoring provenance,
not retrieval locations. Working-tree validation is a separate surface
(`project/instrukt/spec/feature/docs/snippet-validation`).

## Canonical fields

### Index

- **Default view:** `general`, every published domain, plus the project
  snippets of the project the caller stands in — the repository containing the
  working directory, matched by the `project` slug its `instrukt.yaml`
  declares. Outside any project the view holds the published positions only.
- **Filters** (each only narrows):

  | Filter | Effect |
  | --- | --- |
  | `--domain <x>` | every id of that domain (`general` included); repeatable; project snippets and third-party docs drop out; an unknown domain fails non-zero, naming the valid domains |
  | `--areas <t,…>` | only these taxonomy types |
  | `--third-party` | adds the third-party docs section |
  | `--target <slug>` | only snippets the todo delivers or has pending material for (`project/instrukt/spec/feature/docs/pending-documentation`) |
  | `--project <slug>` | lists only that project's snippets |

- **Source:** a listing that names a position opens with that position's
  source, read from publication state, which records the publishing checkout
  per position: the absolute path of the position's folder in the checkout it
  was published from, the published commit, and the repository's remote URL
  (`none` when the repository has no remote). `--domain <x>` opens with the
  source of each named domain (`docs/domain/<x>/`, or `docs/general/` for
  `general`); `--project <slug>`, and the project section of the default view,
  opens with the project's source, its `docs/project/` folder in the
  project's source checkout. This is where the position's snippets are changed;
  retrieval reads the committed published store, not this source path.
- **Entry line:** `<id>: <description>`, followed by `(delivered by: <slugs>)`
  for a pending snippet and `(superseded by: <slugs>)` for a live snippet with
  pending material. Entries are ordered by taxonomy (principle, concept,
  policy, procedure, design, spec, adr), then scope, then id.
- **Shape:** a header naming the available filters, the entries, the
  third-party section when requested, a closing line directing the caller to
  `instrukt docs get` with the ids of interest, and a note explaining the
  pending suffixes and the `--target` view.

### Pending documentation source

`docs index --target <slug>` and `docs get <id> --target <slug>` read the recorded preparation worktree, including unpublished new snippets and pending companions. They never substitute the main checkout for an unresolved preparation workspace. After preparation integration, folded documentation is simply the current contract on main; pending docs do not wait for build delivery.

### Get

- Every id resolves to its published file in the home; a project id resolves
  under `docs/project/<slug>/…`. A project
  id of another project is read from any location; no project flag is needed.
- An unknown id prints an error naming it and exits non-zero; known ids in the
  same call are not rendered.
- Each rendered snippet is a header block (id, type, domain, scope,
  description, source) followed by its body. `source` is the absolute path of
  the snippet's file in the checkout it was published from (for a project
  snippet, its file in its project): the file to edit to change it. A
  third-party doc's header names its source file the same way. The output header lists the required
  reads that were not loaded and must have been read at least once: a snippet
  read by its id, a bare-domain read by its domain name with
  `instrukt docs get --domain <x>`.
- Required reads are resolved transitively, by id. A project snippet's
  required reads follow its own project's snippets and published snippets
  only.
- A snippet's required reads are listed in the order the snippet declares
  them.
- A withheld snippet's required reads are not named.
- A live snippet with pending material, read without `--target`, is followed
  by a notice naming the owning slugs and how to read their view.
- A third-party id renders its body; required-read resolution does not apply.

### Closure

`--closure` takes exactly one snippet id, delivered, and no `--target`. A
closure request naming several ids, or a third-party doc, is refused with an
error saying a closure is read for exactly one snippet. It renders the
snippet, every transitive required read once, and, for each bare-domain read,
that domain's `expanded` set with its own closure, listing the included ids in
its header; a domain read is listed by its domain name. Transitive reads are
listed depth-first in declaration order, each once, at its first occurrence.
For a procedure carrying root frontmatter (`role`, `subscriptions` or
`activation_trigger`), a required read that is pending,
resolves to no readable published snippet, or names a domain without an
`index.yaml` refuses the closure, naming it. For any other snippet such a read
is left out. Triggered-procedure listing lines direct to this form.

### Domain read

`instrukt docs get --domain <x>` renders domain `<x>`'s position index
(`project/instrukt/spec/feature/docs/position-index`); `general` is
read the same way. It takes no ids, no `--closure` and no `--target`: it reads
the current contract.

- **Content, in order:** every `expanded` snippet in full with its closure,
  each snippet once across the whole read; then one listing line per `listed`
  id; then one listing line per procedure of that domain carrying an
  `activation_trigger`.
- **Format:** grouped and formatted exactly as publication renders that
  domain's group in the global agent files
  (`project/instrukt/spec/feature/publication/agent-entrypoints`). For
  an on-demand domain this is the full content its description line in the
  global file stands for, not merely that line.
- Each rendered body passes the read pipeline below, so configured values
  resolve for the caller. A required read of an `expanded` snippet that is
  pending or cannot be read is left out and named in the output header.
- **No index:** for a published domain without `index.yaml`, the read prints
  that the domain has no `index.yaml`, directs the caller to
  `instrukt docs index --domain <x>`, and exits zero.
- An unknown domain, or a reserved name other than `general`, fails non-zero,
  naming the valid domains.

### Listing lines

A listing line stands for one snippet and carries the command an agent runs to
read it. Every rendering that lists snippets uses these forms: the domain read
above, the global agent files and the project root `AGENTS.md`.

| Stands for | Line |
| --- | --- |
| a `listed` snippet | `` - <id>: <description> — `instrukt docs get <id>` `` |
| a procedure with an `activation_trigger` | `` - <id>: <description> — when: <activation_trigger> — `instrukt docs get <id> --closure` `` |

### Read pipeline

Every body `get` renders passes, in order:

1. **Pending gate** — a pending snippet outside the lens is withheld with a
   notice (`project/instrukt/spec/feature/docs/pending-documentation`).
2. **Companion** — with `--target <slug>`, that slug's companion replaces the
   body when it exists.
3. **Planned lens** — planned blocks resolved for the lens; hidden content never executes.
4. **`@config`** — each visible configured-value directive is resolved (below).
5. **Frontmatter** — stripped.
6. **Required reads** — the section is stripped from the body.

Required-read extraction uses each reference link's id text. Rendering
reduces links in Required reads and See Also to that text. Agent-facing output carries ids only,
including closure and domain reads and rendered agent files; link targets
never participate in resolution. Other Markdown links and code examples stay
as authored.

### Configured values

A configured value is the directive `<!-- @config: <path> -->` in a snippet
body. `get` resolves `<path>` in-process through configuration's `get`, with the
same interpolation and the same refusals: an unknown path, and
`runtime.session_id` read without the selected adapter's caller context
(`project/instrukt/spec/feature/configuration/interpolation`).

- It runs no shell and no subprocess. It resolves exactly as
  `instrukt config get <path>` would for the caller: against the effective
  configuration of the project the caller stands in (project over global),
  found by the same project resolution `index` and `get` use for project docs,
  or against the global layer alone outside a project. Project checks come from
  the project's documentation, not an `actions` configuration key.
- It resolves on every read, never cached.
- The value replaces the directive as a fenced block, rendered as
  `instrukt config get` prints it; a structured value prints as YAML.
- A refusal replaces the directive with a visible notice naming the path and
  the reason; `get` prints the whole output and then exits non-zero.

Published snippets carry the directive unresolved; only the reader's `get`
resolves it. Rendered agent files resolve it at publication instead, against
the publishing project's effective configuration with no runtime session,
which is publication's contract.

The snippet format has no directive that runs a command or includes another
file. A leftover `<!-- @exec: … -->` or `<!-- @include: … -->` is never run or
expanded: `get` shows it as inert text, and validation reports it as a finding
(`project/instrukt/spec/feature/docs/snippet-validation`).

### Publication behind main

When the caller stands in a project whose canonical main is ahead of the
commit publication last published for it, `index` and `get` print a warning
naming both commits alongside their normal output. They publish nothing.

## Use cases

### UC-DR1: An agent loads the documentation a task needs

```gherkin
Given an agent acting for a developer works in a project that has published its documentation
When it lists the documentation filtered to the areas and domains its task concerns
And it reads the snippets it selects
Then it receives each selected snippet's current text
And it is told, by id, which required reads it has not yet loaded
And nothing it did not ask for is loaded
```

### UC-DR2: An agent reads a snippet with everything it requires

```gherkin
Given an agent acting for a developer needs a snippet whose required reads name other snippets by id and domains by name
When it reads that snippet as a closure, whether a procedure it is directed to run, a design or an overview
Then it receives the snippet, each of its transitive required reads once, and what the domains they name show in full, in one read
And the snippets' text names no path on the author's computer; only the source lines in the headers name where each snippet is authored
But when a required read of a procedure that carries root frontmatter is still pending or cannot be read
Then the read is refused and names the required read that blocked it
And for a supporting procedure or any other snippet such a read is left out and named
But when it asks for a closure of several items at once
Then the read is refused and says a closure is read for exactly one snippet
```

### UC-DR3: An agent reads instructions that carry values from its own session

```gherkin
Given a published procedure whose text asks for a configured value
And two agents acting for the same developer run in different sessions on one machine
When each agent reads that procedure
Then each receives the text with the value resolved for its own session at the moment of reading
And the published procedure itself still carries the request, not a value
But when the value cannot be resolved
Then the agent receives the text with a notice naming the value and why it could not be resolved, and the read reports failure
But when a snippet tries to run a command instead of asking for a value
Then publication refuses it and names the file
And reading such a snippet runs nothing
```

### UC-DR4: An agent reads a domain on demand

```gherkin
Given an agent's global instructions carry one line for a published domain, naming the command that reads it
When the agent reads that domain
Then it receives every snippet the domain shows in full, each with what it requires and each once
And one line for each snippet the domain lists and each procedure it can be asked to run, each line naming the command that reads it
But when the domain has no index
Then it is told the domain has no index and how to list the domain's snippets instead
```

### UC-DR7: An agent finds where a domain's doctrine is authored

```gherkin
Given a domain is published on a developer's computer from a checkout of its docs repository
When an agent acting for the developer lists that domain's snippets
Then the listing opens with the folder in that checkout the domain is authored in, the commit that was published, and the repository's remote
```

### UC-DR8: An agent finds the file behind a snippet it read

```gherkin
Given an agent acting for a developer has read a published snippet it wants to improve
When it reads the snippet
Then the snippet's header names the file in its publishing checkout that holds it
```

### UC-DR9: An agent reads clickable references as ids

```gherkin
Given a developer's snippets use same-repository links by id in Required reads and See Also
When an agent acting for the developer reads a snippet, its closure or its domain
Then references are shown as plain ids and required reads resolve by id, each once
And the rendered agent instructions carry the same plain-id references
And other Markdown links and code examples remain as authored
```

## Known caveats

- UC-DR5 and UC-DR6 are retired: their goals are part of UC-DR2. Their ids are not reused.

- The planned lens runs before configured values are resolved, so hidden content is never resolved.
- `--project <slug>` lists only that project's snippets. An unknown slug lists nothing and says no such project has published.
- A missing published project file makes `get` name the project and missing
  store path and direct the caller to republish from that project; it never
  falls back to the primary worktree.
- `docs index` entry lines carry no command; its closing line names `instrukt docs get`. Only renderings of a position index carry listing lines.
- A bare-domain required read brings only that domain's `expanded` set, never its `listed` lines or triggered procedures.

## See Also

- [project/instrukt/spec/domain/docs](../../domain/docs.md)
- [project/instrukt/spec/feature/docs/position-index](position-index.md)
- [project/instrukt/spec/feature/docs/pending-documentation](pending-documentation.md)
- [project/instrukt/spec/feature/configuration/interpolation](../configuration/interpolation.md)
- [project/instrukt/spec/feature/publication/agent-entrypoints](../publication/agent-entrypoints.md)
