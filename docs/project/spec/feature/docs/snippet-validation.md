---
description: Structural validation of a project's documentation with `instrukt docs validate` — what is checked, the findings and notices it reports, and how the exit status reflects them.
delivered_by: [instrukt-first-build]
---

# Snippet Validation — Spec

## What it is

`instrukt docs validate [paths]` checks a project's documentation against the
snippet contract (`general/spec/snippet-authoring-schema`), the position
indexes (`project/instrukt/spec/feature/docs/position-index`) and the
effective schema of each snippet
(`project/instrukt/spec/feature/docs/snippet-schema`), and reports every
finding and notice. It is structural only: it judges shape, placement and form, never the
meaning of the prose (`general/principle/intelligence-boundaries`). It reads
the working tree of the repository it runs in, so a developer or agent can
check documentation before committing, and it changes nothing.

## Canonical fields

### Scope of a run

- Without paths: every snippet, position index, position schema,
  companion and third-party doc under the repository's `docs/`.
- With paths: only the named files, plus the cross-snippet checks those files
  take part in.
- Directories whose name starts with `_` are skipped.
- `docs/domain/<x>/workflow/` is a non-snippet directory. Its definitions,
  actions (including `roles:` and `effects:`) and templates are validated
  against the workflow schema, rather than snippet taxonomy and frontmatter.

### Checks

| Family | A finding when |
| --- | --- |
| Identity | the path yields no identity (outside `docs/general/`, `docs/domain/<x>/`, `docs/project/` and `docs/third-party/`; an unknown taxonomy directory; a file directly under `docs/domain/`; a project snippet in a repository whose `instrukt.yaml` declares no `project`); a domain folder carries a reserved name (`general`, `project`); classification fields are authored in frontmatter |
| Frontmatter | `description` is missing; `visibility` is not `public` or `internal`; a procedure-only field sits on another type; `subscriptions` and `activation_trigger` both appear; `parameters` or `dispatch` appear without `role`; `parameters` or `dispatch` are malformed; an `adr` lacks `date` or `number`; `eval` is not a boolean |
| Procedures | with todos enabled, a `role` is absent from the selected flavor’s `workflow/actions.yaml` role list; two procedures declare the same `role`; an entry procedure requires a procedure of its own group folder that carries no root frontmatter |
| Structure | the H1 is missing, not first, or does not end with `— <Type>`; a required H2 section of the effective schema is missing; an H2 outside it appears; `See Also` is not last (`project/instrukt/spec/feature/docs/snippet-schema`) |
| References | `## Required reads` not at H2; a Required reads entry that is not one snippet id or one bare domain name; an id that resolves to no snippet; a bare name that is neither `general` nor a domain of the validated repository or published on the computer, or names a domain without `index.yaml`; a See Also entry starting with an id that resolves to no snippet; a snippet referenced by path (`@…`, or a path to a snippet file) outside code; a `general` or domain snippet requiring a project snippet |
| Required-reads hygiene | a required read already implied by the closure of another required read of the same snippet; the finding names the read that implies it |
| Position index | as `project/instrukt/spec/feature/docs/position-index` lists |
| Schema | a `schema.yaml` finding as `project/instrukt/spec/feature/docs/snippet-schema` lists |
| Pending | `delivered_by` is not a non-empty list of slugs; a companion, planned or change marker names a slug of invalid form; planned markers are inline, nested, unbalanced or overlapping |
| Third-party | `description` missing; a `## Sources` entry that is neither a web URL nor a documentation-service identifier |
| Directives | an `@exec` or `@include` directive in a snippet, companion, third-party doc or master, naming its command or path; a `@config` directive with a `runtime.*` path in an `AGENTS.master.md` or in a snippet rendered into an agent file — reached through an `index.yaml`'s `expanded` closure or `agents` |

A todo slug is checked for form only. Whether it names an existing todo is
checked as well when the `todos` module is on, by asking that module.

With todos enabled, `role` must name a role declared by the flavor workflow.
Core declares the role-lookup interface, as for pending-slug existence; the CLI
wires its todos implementation. With todos off, no role-membership lookup runs.

Validation checks procedure `dispatch` names against the configured runtime
mapping’s startable agents. Configuration supplies that set; docs owns the
frontmatter finding. `requires` is not snippet frontmatter: it is the project
layer's configuration key naming the modules the flavor needs. `docs validate`
and `sync` report a configuration finding when it names a module that is off
under the effective switches (project over global), not the global layer in
isolation.

### Subscription declarations

Subscribed procedures require nonempty shared-matcher patterns, `role` and one
`subscription_home` (`project`, `platform`, `company`, `person`); they cannot also
carry `activation_trigger`. Identity is `home` or `subject` (default home),
attendance `attended` or `unattended` (default attended), delivery `sequential` or
`batched` (default sequential), and interruptible is boolean (default false).
These fields without subscriptions, unknown values and malformed patterns are
findings naming file and field. A project-position subscription answers only its
own project and must declare project home; general/domain positions reach all
homes of the declared kind. Unsupported company/person resolution remains an
execution capability limit rather than conversion to project/platform.

Publication carries the resolved facts, owning project and position unchanged
for routing. Pending declarations are validated structurally but never become
active subscriptions. Authored Required reads supply handling instructions;
delivery metadata adds no hidden dependencies. Runtime settings remain fixed
for an initialized session; changing publication configures only new successors.

### Notices

A notice is reported with its file, a stable code and the value where there is
one, and never fails the run.

- **Unreferenced snippet** — a delivered snippet of the validated repository
  that no `index.yaml` names (through `expanded`, `listed` or `agents`) and no
  snippet names as a required read: it is reachable only through
  `instrukt docs index`. References count from the working tree and from what
  is published on the computer.
- **Domain without `index.yaml`** — as
  `project/instrukt/spec/feature/docs/position-index` states.

### Reporting

- Every finding names its file, a stable finding code and the offending value
  where there is one. All findings are reported in one run; validation does
  not stop at the first.
- `--json` prints the findings as a JSON list instead of text.
- Notices are reported after the findings, apart from them.
- The exit status is non-zero when there is at least one finding, zero
  otherwise; notices never change it.

### Use cases

#### UC-SV1: A developer learns which snippets break the contract before committing

```gherkin
Given a developer has written and changed snippets in their project
When they validate the project's documentation
Then every snippet that breaks the snippet contract is reported with its file and what is wrong
And a snippet without a description is reported rather than left out
And the run reports failure while any finding remains
But when no snippet breaks the contract
Then the run reports success
```

#### UC-SV2: A developer learns which required reads are already implied

```gherkin
Given a developer's snippet requires an overview and also a snippet that overview already requires
When they validate the project's documentation
Then the redundant required read is reported, naming the read whose closure already brings it
And the run reports failure while it remains
```

#### UC-SV3: A developer sees which snippets nothing brings to an agent

```gherkin
Given a developer's project has a snippet that no index names and no snippet requires
When they validate the project's documentation
Then they are told that the snippet is reachable only by listing the documentation
And the run does not fail on that alone
```

#### UC-SV4: A developer learns where a snippet still names another by path

```gherkin
Given a developer's snippet names another snippet by its file path instead of its id
When they validate the project's documentation
Then the reference is reported with the file and the path it names
And a required read whose id names no snippet is reported the same way
```

## Known caveats

- Validation does not judge prose: a citation that no longer holds, a restated
  rule or a misleading description passes.
- A present `## Required reads` that is not the first H2 is a validation finding.
- A prose citation is not resolved: only Required reads and See Also entries are.
- Two required reads whose closures overlap are not redundant; a read is reported only when it lies in another read's closure.
- Mermaid syntax validation is outside the first build.

## See Also

- [project/instrukt/spec/domain/docs](../../domain/docs.md)
- [project/instrukt/spec/feature/docs/position-index](position-index.md)
- [project/instrukt/spec/feature/docs/snippet-schema](snippet-schema.md)
- [project/instrukt/spec/feature/docs/pending-documentation](pending-documentation.md)
