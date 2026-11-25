---
description: Company plugin — a company's repository carries the company's knowledge as a Claude plugin, rendered by instrukt on request from the documents its `docs/index.yaml` selects into `plugins/<key>/` and listed in the repository's plugin catalog; the plugin carries everything it names, its version is raised exactly when its rendering changes, and a check and a validation write nothing.
---
# Company Plugin — Spec

## What it is

A person or an agent that works for a company through Claude without a
checkout of the company's repository receives the company's knowledge as a
Claude plugin. The repository that is the company's home
(`project/instrukt/spec/feature/configuration/company-homes`) is also the
catalog the plugin is installed from. instrukt renders the plugin from the
same index and the same documents as the home's committed `AGENTS.md`
(`project/instrukt/spec/feature/publication/agent-entrypoints`), so both hold
the same knowledge. The plugin holds one skill. Where `AGENTS.md` names a
listed document by its path in the checkout, the plugin carries every
document it names, since it is installed without the repository. The plugin
is rendered when a caller asks for it, and its version is raised exactly when
its rendering changes.

## Canonical fields

### What the plugin holds

The plugin is the folder `plugins/<key>/` at the repository's root, `<key>`
being the company's key. It conforms to Claude's plugin, skill and plugin
catalog formats (See Also), which decide where each of its files stands.

- **The manifest** names the plugin by the company's key and carries its
  version, a description and the company as its author.
- **One skill**, named by the company's key. Its main file carries the
  skill's name and description, the name people write the company by as its
  title, the company passage
  (`project/instrukt/spec/feature/publication/agent-entrypoints`, The company
  passage), and each `expanded` document in full, in the index's order, with
  its required reads, each before the document that requires it and each
  once.
- **Each other document** the index selects or a required read reaches is a
  file of its own inside the skill, under one folder of the skill at the
  document's path relative to `docs/`. The main file names each `listed`
  document by one line: that file's path inside the skill and the document's
  `description`. A document that requires another names it by where the
  plugin carries it.
- **Every document once.** Each document the index selects, and each
  document their required reads reach, is carried exactly once: in the main
  file where an `expanded` document brings it in, otherwise as a file of its
  own.
- **References between documents.** A reference in one company document's
  text to another, in each form the company's knowledge admits
  (`project/instrukt/spec/feature/publication/agent-entrypoints`, The
  company's knowledge stands alone), names in the plugin where the plugin
  carries that document.
- **The descriptions are instrukt's.** The plugin's and the skill's
  descriptions say whose knowledge the plugin holds and when an agent uses
  the skill. They take the company's name and nothing else: no document and
  no index replaces, extends or removes them.
- **Each file opens as its format requires.** No file of the plugin carries
  the generated line of an instruction file
  (`project/instrukt/spec/feature/publication/agent-entrypoints`, The
  generated line).

### What the documents may say

The index and the documents are read as the home's `AGENTS.md` rendering
reads them, and every finding of that rendering is a finding here
(`project/instrukt/spec/feature/publication/agent-entrypoints`, The company's
knowledge stands alone; What the index may say). The plugin adds these, each
naming the document and what is wrong:

- **The index.** A repository without `docs/index.yaml` holds no knowledge
  to render: that is a finding naming the file, for a rendering, a check and
  a validation alike. So does a repository whose root `instrukt.yaml` makes
  it a project
  (`project/instrukt/spec/feature/publication/agent-entrypoints`, What makes
  a repository a company home): that is a finding naming that file.
- **Nothing the plugin names lies outside it.** A reference in a document's
  text to a company document the plugin does not carry — one the index does
  not select and no required read reaches — or to a file under `docs/` that
  is no company document is a finding naming the document and the reference.
- **Frontmatter.** Frontmatter that is not a YAML mapping, in any document
  the plugin carries, is a finding naming the document and why.
- **Visibility.** A company document's frontmatter may carry `visibility`,
  `internal` or `public`; a document without it is `internal`, and any other
  value is a finding. The plugin holds every document whatever its
  visibility. A `public` document that requires or references an `internal`
  one is a finding naming both.

With a finding, no file is written or removed.

### The version

- The manifest's version is three numbers. In a repository that holds no
  manifest for the plugin, the rendering carries instrukt's first version.
- Where the repository holds one, its version is kept when the plugin's
  rendering equals what `plugins/<key>/` holds, file for file, and its last
  number is raised by one otherwise. instrukt changes only the last number:
  the first two are whatever the manifest in the repository holds.
- Rendering twice in a row changes nothing the second time.
- A manifest in the repository that cannot be read, or whose version is not
  three numbers, is a finding naming it.
- A file git ignores is no part of what `plugins/<key>/` holds: it is neither
  compared nor removed.

### What the rendering is a function of

The plugin and its catalog entry are a function of the repository's index and
documents, the company's key, the name people write the company by, the
plugin the repository already holds, and the rendering instrukt release.
Nothing else from the computer enters: no home, no publication, no
configuration layer and no environment. A fresh clone and the binary are
enough, and a home and a clone of the repository render the same bytes.

The caller gives the key and the name with every request, for a rendering, a
check and a validation alike. instrukt takes neither from the repository's
name, the folder it lies in, a settings document or the environment.

- **The key** follows the grammar of a company's key
  (`project/instrukt/spec/feature/configuration/company-homes`).
- **The name** is one line of text that is not blank. Whatever characters it
  holds, each file that carries it stays valid in its format.
- A request without one of them, or with one that breaks its form, is a usage
  error: nothing is read.

### The catalog entry

`.claude-plugin/marketplace.json` at the repository's root is the
repository's plugin catalog. A rendering sets the company's plugin's entry in
it and nothing else:

- The entry names the plugin by the company's key, gives the plugin's folder
  as its source, in the form Claude's catalog format gives a folder of the
  catalog's own repository (See Also), and carries the plugin's description.
  It carries no version of instrukt's: the plugin's version is the
  manifest's alone.
- An entry already listed under that name receives those values and keeps
  every other field it holds; without one, the entry is added after the
  others.
- Every other entry, and every other part of the catalog, keeps its value and
  its place.
- The catalog is compared by its values, never by its bytes. Where the entry
  already holds those values, the file is left byte for byte, whatever its
  formatting. Where it does not, the catalog is written as one JSON document:
  every member in the order it stood in, indented by two spaces, ending in
  one newline.
- A repository without a catalog is a finding naming the file. instrukt
  creates none: a catalog's own name and owner are its repository's. A
  catalog that cannot be read as one is a finding, and is left as it is.

### Rendering, checking and validating

A caller asks for one of three things in a company's repository:

- **A rendering** writes each file of the plugin whose content differs,
  removes each file under `plugins/<key>/` that the rendering does not hold,
  and sets the catalog entry. It reports the plugin's version and each file
  it changed. The folder is instrukt's: it is written whole, whatever it
  held. A rendering stages and commits nothing; committing what it changed is
  its caller's.
- **A check** writes nothing. It names each file whose content in the working
  tree differs from the rendering, each file a rendering would remove and
  each finding, and exits non-zero when it names any.
- **A validation** writes nothing and compares no file. It reports each
  finding and exits non-zero only when there is one, so it answers for a
  branch whose plugin is behind its documents.

The three read the same repository and report the same findings. Nothing is
read, written or removed through a symbolic link: one at or above
`plugins/<key>/`, anywhere inside it, or at or above the catalog is a finding
naming it. The command that asks for each of the three is the publication
context's (`project/instrukt/spec/domain/publication`).

The `pre-commit` section, a sync, a start and a tick render no plugin, and
rendering a repository's `AGENTS.md` renders none. A commit on a branch
carries the plugin its base held.

## Use cases

### UC-CP1: An operator gives people who work with Claude the company's own knowledge

```gherkin
Given an operator keeps a company's knowledge as documents in the company's repository, with an index that selects some in full and lists others
And the repository holds a plugin catalog that lists other plugins
When the operator has the company's plugin rendered
Then the repository holds the company's plugin, whose skill carries the fixed company passage and each selected document in full
And the plugin carries each listed document as a file of its own, named in the skill with its description
And nothing the plugin names lies outside it
And the catalog lists the company's plugin by its folder in the repository, with every other entry as it was
But when a document refers to something outside the company's own documents
Then the rendering reports the document and the reference, and no file is changed
```

### UC-CP2: An operator publishes a new version of the company's plugin exactly when its knowledge changed

```gherkin
Given a company's repository holds the plugin rendered from its documents
When the operator has the plugin rendered again and no document changed
Then no file is changed and the plugin's version is as it was
When the operator changes a document and has the plugin rendered
Then the plugin holds the change and its version is one step higher
And rendering it once more changes nothing
```

### UC-CP3: An operator learns before a change merges whether the company's knowledge still renders

```gherkin
Given a company's repository whose plugin was rendered from the documents its default branch holds
And a branch that changes a document without rendering the plugin
When the operator has the branch's documents validated
Then the validation passes although the branch's plugin is behind its documents
And no file is changed
But when the change makes a public document depend on an internal one
Then the validation fails naming both documents, and no file is changed
```

### UC-CP4: An operator learns whether the plugin the repository holds still matches its documents

```gherkin
Given a company's repository holds the plugin rendered from its documents
When the operator has the plugin checked
Then the check passes and no file is changed
When a document changes and the operator has the plugin checked again
Then the check fails naming each file of the plugin that is behind the documents
And no file is changed
```

## Known caveats

<!-- planned-change:company-home-integration -->
- One plugin is rendered, and it holds every document: a document's
  `visibility` selects nothing.
- A version is raised against what the working tree holds. Two branches that
  each render the plugin and are then merged can carry one version for
  different content; a rendering is asked for where the catalog is installed
  from, the repository's default branch.
- Between the merge of a change to the knowledge and the rendering asked for
  on the default branch, the catalog serves the previous plugin.
- The rendering depends on the instrukt release that renders it. A release
  that renders a plugin differently raises that plugin's version at its next
  rendering although no document changed.
- instrukt and the shared workflows repository's publisher
  (`project/instrukt/policy/shared-workflows`) each write `plugins/<key>/`
  whole and each raise its version; a repository whose plugin both render
  receives each renderer's bytes in turn.
- When and whether a Claude client takes a new copy of an installed plugin is
  Claude's (See Also).
- What Claude does with a skill's main file before an agent reads it, and how
  much of it Claude keeps in an agent's context, is Claude's (See Also).
  Text of an `expanded` document in a form Claude replaces or runs there is
  not delivered as written. A document the index lists stays out of the main
  file.
- Whether Claude accepts a plugin under the company's key is Claude's (See
  Also): the key is held to the grammar of a company's key and to nothing
  else.
- A `version` an operator sets on the plugin's catalog entry stays as it is,
  and so does not follow the manifest's.
<!-- change:company-home-integration -->
- One plugin is rendered, and it holds every document: a document's
  `visibility` selects nothing.
- A version is raised against what the working tree holds. Two branches that
  each render the plugin and are then merged can carry one version for
  different content; a rendering is asked for where the catalog is installed
  from, the repository's default branch.
- Between the merge of a change to the knowledge and the rendering asked for
  on the default branch, the catalog serves the previous plugin. A company
  home integrated through its standing integrator has no such interval: each
  integration candidate carries its rendering
  (`project/instrukt/spec/feature/workflow/integration-gate`, A company home's
  candidate).
- The rendering depends on the instrukt release that renders it. A release
  that renders a plugin differently raises that plugin's version at its next
  rendering although no document changed.
- instrukt and the shared workflows repository's publisher
  (`project/instrukt/policy/shared-workflows`) each write `plugins/<key>/`
  whole and each raise its version; a repository whose plugin both render
  receives each renderer's bytes in turn. In a company home integrated through
  its standing integrator, that publisher is a second writer of the default
  branch beside the integrator (`general/policy/commits`).
- When and whether a Claude client takes a new copy of an installed plugin is
  Claude's (See Also).
- What Claude does with a skill's main file before an agent reads it, and how
  much of it Claude keeps in an agent's context, is Claude's (See Also).
  Text of an `expanded` document in a form Claude replaces or runs there is
  not delivered as written. A document the index lists stays out of the main
  file.
- Whether Claude accepts a plugin under the company's key is Claude's (See
  Also): the key is held to the grammar of a company's key and to nothing
  else.
- A `version` an operator sets on the plugin's catalog entry stays as it is,
  and so does not follow the manifest's.
<!-- /planned-change:company-home-integration -->

## See Also

- [project/instrukt/spec/feature/publication/agent-entrypoints](agent-entrypoints.md) — the company index, the company's documents, their findings and the company passage.
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md) — the company's home and its key.
- [project/instrukt/spec/domain/publication](../../domain/publication.md) — the publication context.
- Claude's formats and update rule, from its vendor: plugin catalogs https://code.claude.com/docs/en/plugin-marketplaces; plugin manifests and versions https://code.claude.com/docs/en/plugins-reference; skills https://code.claude.com/docs/en/skills.
