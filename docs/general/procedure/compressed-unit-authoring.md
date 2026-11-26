---
description: Write or renew the compressed unit of a source an entrypoint rendering holds in full — learn offline which units are stale, missing or orphaned, compress each named source's current contract into flat, deduplicated ASCII rules in the shared notation, declare the digest it was written from, and leave every other unit untouched.
visibility: 'public'
delivered_by: [compressed-global-entrypoints]
---

# Compressed Unit Authoring — Procedure

## Required reads

- [general/spec/agent-shorthand](../spec/agent-shorthand.md)
- [general/spec/snippet-authoring-schema](../spec/snippet-authoring-schema.md)

## Goal

Give each source that needs one a current compressed unit: a dense
counterpart an entrypoint rendering holds in the source's place while the
source is unchanged. The unit keeps every behavioral invariant of its source
with a one-clause reason, in the compressed-unit notation
(`general/spec/agent-shorthand`), so agents receive the whole selection within
their runtime's instruction-size limit and reach the full prose by the
source's name.

## Preconditions

- A checkout of the repository that holds the source, and the instrukt
  binary. No home, publication, configuration or network is needed.
- Authority to commit there: an authoring change that alters the source, or
  the repository's post-merge automation acting for its maintainers.

## Steps

1. **Ask what needs a unit.** Run `instrukt docs units` in the checkout
   (`--json` for a program). Exit 0 says there is nothing to compress: stop.
   Exit 3 names each source whose unit is stale or missing — its id or
   company document path, its file, where its unit belongs and the digest a
   renewed unit declares — and each orphaned unit. Exit 1 also names
   findings; a finding on a unit is repaired by renewing that unit here.
   Work only on what it names. A unit it does not name is current and stays
   byte for byte as it is: renewing it spends a model call and moves the eval
   fingerprint while its source has not changed.

2. **Renewing for a delivery that is still pending.** When the source change
   is pending documentation of a todo, the unit is written for the source as
   that todo delivers it. In a throwaway clone of the todo's work branch run
   `instrukt docs fold <slug>`, then `instrukt docs units`; write from the
   folded source and declare the digest named there. On the work branch the
   unit reads as stale, and the renderings hold the source in prose, until
   the delivery folds.

3. **Read the source's current contract, from its file.** Open the file
   `instrukt docs units` named, not a rendering of it: a rendering resolves
   configured values, and a unit carries every `@config` directive exactly as
   its source writes it. Compress the body after the frontmatter, without
   what a `planned` block adds or the replacement side of a `planned-change`
   block, because pending material is not yet contract.

4. **Extract the rules, then deduplicate them flat.** List every behavioral
   invariant the source states: each obligation, prohibition, permission,
   condition, exception, scope limit, threshold, owner and order. A source
   states many of them more than once — a rule, its implication, its
   rationale and its enforcement are often one rule. Merge each such group
   into one rule line carrying the reason once. Keep the result one flat
   list in source order; keep a heading only where the source has parts a
   reader must tell apart, such as a procedure's numbered steps. Paraphrasing
   section by section keeps the source's repetition and stays near its
   length; flat, deduplicated rules typically reach a sixth to a fifth of it.

5. **Write each rule in the notation.** One line per rule, its keyword
   first, in telegraphic words, then ` -- ` and the reason in one clause
   naming the invariant protected or the failure prevented; a fact the rules
   rest on is a line of its own, `<term> = <meaning>`
   (`general/spec/agent-shorthand`, Compressed-unit notation). Ids, commands, paths, code spans and `@config`
   directives stand verbatim, in backticks as in the source. Leave out what
   changes no behavior: reasoning beyond the one clause, examples that only
   illustrate a rule already kept, history and See Also lists. Carry no
   reference, required read or directive the source does not carry.

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

   The rationale became the reason of the rule it explains, and the
   enforcement a rule of its own; nothing of the source is lost and nothing
   is added.

6. **Declare where it came from.** Write the unit at the place
   `instrukt docs units` named, with frontmatter of exactly two keys:
   `source_sha256`, the digest it named for the source, and `model`, the
   identifier of the model writing the unit (`general/spec/snippet-authoring-schema`).
   Write plain ASCII and nothing else in the frontmatter.

7. **Check it against its source yourself.** Read the unit beside the source
   rule by rule: every invariant of the source has a line, and no line says
   what the source does not. instrukt checks a unit's structure and never its
   meaning (`general/principle/intelligence-boundaries`); this reading is the
   only check that the unit keeps its source's rules.

8. **Remove each orphaned unit named** — its source no longer exists or no
   entrypoint index inlines it — because an orphan is never rendered and
   only misleads a reader of the folder.

9. **Confirm and commit.** Run `instrukt docs units` again: the sources you
   renewed are no longer named — in the folded clone, for a pending
   delivery — and no finding names your units. Commit the
   units with the repository's renderings — the `pre-commit` section renders
   a root `AGENTS.md` from the commit; where no hook runs, run
   `instrukt docs render` first, and render a company's plugin as its
   maintainers do.

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
- A rule resists compression without losing a condition: keep the condition.
  A longer unit is correct; a lossy one stands in for its source while
  saying less than it.

## Discipline

- One unit per source, and only the units `instrukt docs units` names.
  Rewriting a current unit changes what agents receive without any change of
  its source.
