---
description: Structural validation of a project's snippets with `instrukt docs validate` — what is checked, how findings are reported, and how the exit status reflects them.
delivered_by: [instrukt-first-build]
---

# Snippet Validation — Spec

## What it is

`instrukt docs validate [paths]` checks a project's documentation against the
snippet contract (`general/spec/snippet-authoring-schema`) and reports every
finding. It is structural only: it judges shape, placement and form, never the
meaning of the prose (`general/principle/intelligence-boundaries`). It reads
the working tree of the repository it runs in, so a developer or agent can
check documentation before committing, and it changes nothing.

## Canonical fields

### Scope of a run

- Without paths: every snippet, fragment, baseline manifest, companion and
  third-party doc under the repository's `docs/`.
- With paths: only the named files, plus the cross-snippet checks those files
  take part in.
- Directories whose name starts with `_` are skipped.
- `docs/global/<domain>/workflow/` is a non-snippet directory. Its definitions,
  actions (including `roles:` and `effects:`) and templates are validated
  against the workflow schema, rather than snippet taxonomy and frontmatter.

### Checks

| Family | A finding when |
| --- | --- |
| Identity | the path yields no identity (unknown taxonomy directory, a published domain segment missing, a local snippet in a repository whose `instrukt.yaml` declares no `project`); classification fields are authored in frontmatter |
| Frontmatter | `description` is missing; `visibility` is not `public` or `internal`; a procedure-only field sits on another type; `subscriptions` and `activation_trigger` both appear; `parameters` or `dispatch` appear without `role`; `parameters` or `dispatch` are malformed; an `adr` lacks `date` or `number`; `eval` is not a boolean |
| Procedures | with todos enabled, a `role` is absent from the selected flavor’s `workflow/actions.yaml` role list; two procedures declare the same `role`; an entry procedure requires a procedure of its own group folder that carries no root frontmatter |
| Structure | the H1 is missing, not first, or does not end with `— <Type>`; a required H2 section is missing; an H2 the taxonomy does not allow appears; `See also` is not last |
| References | an `@` reference outside `## Required reads`; `## Required reads` not at H2; a reference in a form other than the published-home form (`@~/.instrukt/docs/…` or its home-expanded absolute form), a domain bundle, or `@docs/local/…`; a reference that resolves to no file; a published snippet referencing local material |
| Baselines | a manifest line that is not a single `@` reference; a manifest out of taxonomy order; a manifest naming a snippet its domain's bundle already carries |
| Pending | `delivered_by` is not a non-empty list of slugs; a companion, planned or change marker names a slug of invalid form; planned markers are inline, nested, unbalanced or overlapping |
| Third-party | `description` missing; a `## Sources` entry that is neither a web URL nor a documentation-service identifier |

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

### Reporting

- Every finding names its file, a stable finding code and the offending value
  where there is one. All findings are reported in one run; validation does
  not stop at the first.
- `--json` prints the findings as a JSON list instead of text.
- The exit status is non-zero when there is at least one finding, zero
  otherwise.

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

## Known caveats

- Validation does not judge prose: a citation that no longer holds, a restated
  rule or a misleading description passes.
- A present `## Required reads` that is not the first H2 is a validation finding.
- Mermaid syntax validation is outside the first build.

## See Also

- local/projects/instrukt/spec/domain/docs
- local/projects/instrukt/spec/feature/docs/pending-documentation
