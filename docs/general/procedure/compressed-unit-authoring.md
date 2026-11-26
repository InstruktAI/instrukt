---
description: Write or renew the compressed unit of a source an entrypoint rendering holds in full — learn offline which units are stale, missing or orphaned, compress each named source's contract into flat, deduplicated rules in the compressed-unit notation, declare the digest it was written from, and leave every other unit untouched.
visibility: 'public'
---

# Compressed Unit Authoring — Procedure

## Required reads

- [general/spec/agent-shorthand](../spec/agent-shorthand.md)
- [general/spec/snippet-authoring-schema](../spec/snippet-authoring-schema.md)

## Goal

Give each source that needs one a current compressed unit: a dense
counterpart an entrypoint rendering holds in the source's place while the
source is unchanged, keeping every behavioral invariant of its source in the
compressed-unit notation (`general/spec/agent-shorthand`), so agents receive
the whole selection within their runtime's instruction-size limit and reach
the full prose by the source's name.

## Preconditions

- A checkout of the repository that holds the source, and the instrukt
  binary. No home, publication, configuration or network is needed.
- Authority to commit there: an authoring change that alters the source, in
  that change's own worktree, or the repository's post-merge automation
  acting for its maintainers.

## Steps

1. **Ask what needs a unit.** Run `instrukt docs units` in the checkout
   (`--json` for a program). Exit 0: there is nothing to compress; stop.
   Exit 3: it names each source whose unit is stale or missing — its id or
   company document path, its file, where its unit belongs and the digest a
   renewed unit declares — and each orphaned unit. Exit 1: it also names
   findings; a finding on a unit is repaired by renewing that unit here.
   Work only on what it names, and leave every other unit byte for byte as
   it is.

2. **For a source a todo changes, ask as the todo delivers it.** When the
   source carries pending documentation of the todo you work for, run
   `instrukt docs units --target <slug>` in that todo's worktree instead: it
   reads each source with that todo's pending documentation applied and
   names the digest a unit for the delivered source declares. Plain
   `instrukt docs units` reads such a unit as stale, and the renderings hold
   that source in prose, until the delivery folds.

3. **Read the source's contract from its file.** Open the file
   `instrukt docs units` named, not a rendering of it: a unit carries every
   `@config` directive exactly as its source writes it. Compress the body
   after the frontmatter as the contract stands: without what a `planned`
   block adds and with the current side of each `planned-change` block — or,
   for step 2, with that todo's `planned` content, the replacement side of
   its `planned-change` blocks and its companion file in place of the
   parent. `instrukt docs get <id> --target <slug>` shows the same delivered
   text with configured values resolved.

4. **Extract the rules, then deduplicate them flat.** List every behavioral
   invariant the source states: each obligation, prohibition, permission,
   condition, exception, scope limit, threshold, owner and order. A rule, its
   implication, its rationale and its enforcement are often one rule stated
   several times: merge each such group into one rule line carrying its
   reason once. Keep one flat list in source order, with a heading only where
   the source has parts a reader must tell apart, such as a procedure's
   ordered steps. Flat, deduplicated rules typically reach a sixth to a fifth
   of the source's length; a section-by-section paraphrase stays near it.

5. **Write the unit in the notation.** Write every line as
   `general/spec/agent-shorthand` (Compressed-unit notation) defines it, the
   whole unit in plain ASCII. Leave out what changes no behavior: reasoning
   beyond each rule's one-clause reason, examples that only illustrate a rule
   already kept, history and See Also lists. Carry no reference, required read
   or directive the source does not carry.

   A worked exemplar — a policy source:

   ```text
   ## Rules

   - **Read your role's commands before work.** Before doing anything, run
     `instrukt --help` and read the commands your role has. An action that
     help does not offer you goes back to the dispatcher in your output,
     which can change the assignment.

   ## Rationale

   Reading the help first means work is not planned around a command the
   role cannot run.

   ## Enforcement

   Reviewers treat work begun without reading the role's commands as a
   process miss.
   ```

   Its unit:

   ```text
   ---
   source_sha256: <the digest instrukt docs units named>
   model: <the model writing the unit>
   ---
   - MUST run `instrukt --help` and read own role's commands before any work -- no plan rests on a command the role cannot run.
   - MUST return an action help does not offer to the dispatcher, in output -- dispatcher can change the assignment.
   - work begun without reading them = process miss, for reviewers.
   ```

   The rationale became the reason of the rule it explains and the
   enforcement a fact line of its own; nothing of the source is lost and
   nothing is added.

6. **Declare where it came from.** Write the unit at the place
   `instrukt docs units` named, with frontmatter of exactly two keys:
   `source_sha256`, the digest it named for the source, and `model`, the
   identifier of the model writing the unit
   (`general/spec/snippet-authoring-schema`).

7. **Check it against its source yourself.** Read the unit beside the source
   rule by rule: every invariant of the source has a line, and no line says
   what the source does not. instrukt checks a unit's structure and never its
   meaning (`general/principle/intelligence-boundaries`).

8. **Remove each orphaned unit** `instrukt docs units` named.

9. **Confirm and commit.** Run `instrukt docs units` again, with
   `--target <slug>` for step 2: the sources you renewed are no longer named
   and no finding names your units. Commit the units with the repository's
   renderings — the `pre-commit` section renders a root `AGENTS.md` from the
   commit; where no hook runs, run `instrukt docs render` first, and render a
   company's plugin as its maintainers do.

## Outputs

- A current unit for each source `instrukt docs units` named, at the place
  it named, and no orphaned unit, committed with the renderings that hold
  them.

## Recovery

- `instrukt docs units` exits 1 on a unit you wrote: repair what the finding
  names — a frontmatter key other than the two, a malformed digest, a
  configured-value directive added or dropped, a reference the source lacks,
  `@exec` or `@include` — and run it again.
- The digest it names moved while you worked: the source changed; start
  again from step 1.
- A rule resists compression without losing a condition: keep the condition,
  however long the unit grows.

## Discipline

- One unit per source, and only the units `instrukt docs units` names.
