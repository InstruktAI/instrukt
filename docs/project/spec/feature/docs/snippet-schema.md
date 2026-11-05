---
description: Snippet shapes — the root rules built into the binary, a position's `schema.yaml` adding sections per folder, the effective schema a snippet is held to, findings that name the file imposing a rule, and `instrukt docs schema <folder>`.
delivered_by: [instrukt-first-build]
---

# Snippet Schema — Spec

## What it is

A snippet's shape comes from two places. The binary carries the root rules: the
seven types, their sections and the generic rules every snippet follows; they
are fixed. A position may add sections for the snippets under a folder with one
`schema.yaml` at its root. A snippet is held to its effective schema — the root
rules for its type, then its own position's additions for its folder — and
nothing else: inheritance never crosses positions or repositories, so a domain
or a project validates identically on every computer. An author learns a
folder's effective schema with `instrukt docs schema <folder>`, and every
finding on a section names the file that imposed the rule. Validation applies
the schema (`project/instrukt/spec/feature/docs/snippet-validation`).

## Canonical fields

### Root rules

Built into the binary; not configurable.

| Type | Required sections | Optional sections |
| --- | --- | --- |
| principle | Principle, Rationale, Implications, Tensions | |
| concept | What, Why | |
| policy | Rules, Rationale, Scope, Enforcement, Exceptions | |
| procedure | Goal, Preconditions, Steps, Outputs, Recovery | Discipline |
| design | Purpose, Inputs/Outputs, Invariants, Primary flows, Failure modes | |
| spec | What it is, Canonical fields | Allowed values, Known caveats |
| adr | Context, Decision, Consequences | Alternatives Considered, Status |

Generic rules, for every type:

- The H1 is the first content line and ends with `— <Type>` of the snippet's
  taxonomy directory.
- Frontmatter carries `description`; the other fields and where each is
  allowed are the snippet model's (`project/instrukt/spec/domain/docs`).
- `## Required reads` is optional and, when present, the first H2; it holds
  only references (`project/instrukt/spec/feature/docs/progressive-retrieval`).
- `## See Also` is optional and, when present, the last H2.
- Sections are H2 headings matched by their exact title. Headings below H2
  inside a section are free.

### `schema.yaml`

- At most one per position, at its root: `docs/general/schema.yaml`,
  `docs/domain/<x>/schema.yaml`, `docs/project/schema.yaml`. A `schema.yaml`
  anywhere else is a finding.
- Each key is a folder path relative to the position root, in whole segments,
  starting with a taxonomy directory (`spec/feature`, `spec/domain`,
  `procedure/lifecycle`).
- Each value may carry `required` and `optional`, each a list of H2 section
  titles added for the snippets under that path, its subfolders included.

```yaml
spec/feature:
  required: [Use cases]
spec/domain:
  required: [Ownership boundary, Governing designs and decisions]
```

- A position without `schema.yaml` follows the root rules only.
- Once a project has `docs/project/schema.yaml`, it is the project's own;
  instrukt never rewrites it. Its creation on adoption is `instrukt init`'s
  (`project/instrukt/spec/feature/repository/project-initialization`).

### The effective schema

For a snippet of type `T` in folder `F` of a position:

1. Start from the root rules for `T`.
2. Apply every key of that position's `schema.yaml` whose path is `F` or an
   ancestor of `F`, shortest path first; each adds its `required` and
   `optional` sections.
3. A section required at any step is required; every other section named at
   any step is optional. Any other H2 is not allowed.

A path may add sections; it may never drop one. A key that lists as optional a
section the root rules or a shorter path require is a finding, naming both.

Required sections follow the order of the steps that impose them: root order
first, then each key's list in order. Validation checks that the required
sections are present, not their order beyond the generic rules.

### Findings

Every section finding names the snippet's file, the section, and what imposed
the rule: `instrukt root rules`, or the `schema.yaml` file and its path key.

| A finding when |
| --- |
| a required section is missing; it names the root rules or the `schema.yaml` and key that require it |
| an H2 outside the effective schema appears; it names the folder and directs to `instrukt docs schema <folder>` |
| `schema.yaml` cannot be parsed, or a value carries a key other than `required` and `optional` |
| a path key does not start with a taxonomy directory, or is not whole segments |
| one key lists a section both as required and as optional |
| a key drops a section the root rules or a shorter path require |

### `instrukt docs schema <folder>`

- `<folder>` is a folder of the invoking repository's working tree inside a
  position; it need not exist yet. A folder outside every position fails
  non-zero, naming the positions.
- It reads the root rules and that position's `schema.yaml` from the working
  tree; nothing published on the computer changes the result.
- It prints, for each type the folder can hold — the one type of its taxonomy
  directory, or every type at a position root — the required sections and the
  optional sections, each followed by what imposes it (`instrukt root rules` or
  `<schema.yaml> <key>`), then a skeleton: frontmatter with `description`, the
  H1 `# <Title> — <Type>`, and every required section as an H2 in order.
- A `schema.yaml` with findings makes it print those findings and exit
  non-zero.

### Use cases

#### UC-SH1: An author learns the shape a folder's snippets must take

```gherkin
Given a developer is about to write a snippet in a folder of their project
When they ask for that folder's schema
Then they see, for the type that folder holds, every required and every optional section
And for each section what imposes it, the built-in rules or their project's schema with its folder path
And a skeleton they can start the snippet from
```

#### UC-SH2: A publisher adds a section to a folder's snippets

```gherkin
Given a developer's position schema requires an extra section for the snippets under one folder
When they validate the project's documentation
Then every snippet under that folder without the section is reported, naming the section, the schema file and the folder path that requires it
And snippets outside that folder are not held to it
But when their schema makes optional a section the built-in rules or a shorter folder path require
Then the schema itself is reported, naming the section and the rule it would drop
```

#### UC-SH3: A project's snippets are judged the same on every computer

```gherkin
Given a developer's project carries its own schema
And their computer publishes a domain whose schema requires other sections
When they validate the project's documentation
Then the project's snippets are held only to the built-in rules and the project's own schema
And a teammate validating the same commit on another computer gets the same findings
```

## Known caveats

- Validation checks the presence of sections, never their content or their order beyond Required reads first and See Also last.
- A schema adds sections only; it cannot add frontmatter fields, types or taxonomy directories.

## See Also

- [project/instrukt/spec/domain/docs](../../domain/docs.md)
- [project/instrukt/spec/feature/docs/snippet-validation](snippet-validation.md)
- [project/instrukt/spec/feature/repository/project-initialization](../repository/project-initialization.md)
