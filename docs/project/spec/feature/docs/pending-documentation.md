---
description: Pending documentation — intended changes written before the build, flagged in the index, withheld from the default read, and shown through a todo's lens with `--target`.
delivered_by: [instrukt-first-build]
---

# Pending Documentation — Spec

## What it is

Documentation leads the code: the intended contract is written before the
work that delivers it. Pending documentation keeps that intended contract
beside the current one without misleading anyone who reads the current one.
Every pending carrier names the todo that will deliver it. A default read
shows the current contract; a read through one todo's lens shows the
documentation as it will be once that todo delivers. How authors write the
carriers is `general/procedure/doc-snippet-authoring`; this spec states what
readers get.

## Canonical fields

### Carriers

| Carrier | Means | Default read | Read with `--target <slug>` |
| --- | --- | --- | --- |
| `delivered_by: [slugs]` on a snippet | a new snippet, not yet delivered | withheld with a notice | the body, when `<slug>` is listed |
| `<stem>.<slug>.md` beside a live snippet | the complete next version | the live snippet | the companion's body |
| empty `<stem>.<slug>.md` | the snippet is retired | the live snippet | an empty view |
| `planned:<slug>` block | an addition | dropped | kept, markers removed |
| `planned-change:<slug>` … `change:<slug>` … block | a replacement or removal | the current side | the replacement side |

A lens shows only its own todo's material: other todos' planned blocks are
resolved as in a default read.

### Rules

- **Superseded by is derived.** A live snippet's `superseded_by` is the set of
  slugs named by its companions and planned blocks. It is never authored.
- **The index flags, never hides.** A pending snippet's entry ends with
  `(delivered by: <slugs>)`; a live snippet with pending material ends with
  `(superseded by: <slugs>)`.
- **`index --target <slug>`** lists only the snippets the todo delivers or
  has pending material for.
- **The withheld notice.** `get` of a pending snippet outside its lens prints
  the snippet's header with a notice that its body is withheld until delivery,
  naming the delivering slugs and the `--target` read; it never prints the
  body, and it never prints nothing.
- **The awareness notice.** `get` of a live snippet with pending material,
  without `--target`, prints the current body followed by a notice naming the
  owning slugs and the `--target` read.
- **A lens that does not reach.** `get --target <slug>` of a pending snippet
  that `<slug>` neither delivers nor reworks is withheld like a default read.
- **Slug checks.** A slug is checked for form during validation; whether it
  names an existing todo is checked only when the `todos` module is on.
- **Closures stay current.** A closure never includes a pending required read
  (`project/instrukt/spec/feature/docs/progressive-retrieval`).
- **Indexes name delivered snippets.** An `index.yaml` naming a pending
  snippet is a finding
  (`project/instrukt/spec/feature/docs/position-index`); a named live
  snippet with pending material is rendered as its current contract.

### Delivery fold

`instrukt docs fold <slug>` applies precisely that delivery's pending carriers:
clear its new-snippet delivery marker; replace a live parent with its non-empty
companion and remove that companion; remove an empty retirement companion and
its parent; unwrap its planned additions; apply its planned replacements or
removals and clear their markers. Preserve unrelated current and pending material.
The integrator folds in the integration candidate before verification and
includes the folded content in the exact commit verified.

## Use cases

### UC-PD1: A developer documents an intended change without misleading current readers

```gherkin
Given a developer has written the intended contract of a change before building it, naming the todo that will deliver it
When an agent acting for the developer reads that documentation without naming the todo
Then it reads the current contract
And it is told that pending changes exist and which todo owns them
And the listing marks the pending material with that todo
```

### UC-PD2: An agent building a todo reads the documentation as it will be after delivery

```gherkin
Given a todo has pending documentation: a new snippet, a full next version of a live snippet, and additions inside another
When an agent acting for a developer works on that todo and reads those snippets through its lens
Then it reads the new snippet, the next version, and the live snippet with the additions in place
And it sees no other todo's pending material
But when it reads a pending snippet its todo does not deliver
Then the body is withheld with a notice
```

## Known caveats

- A withheld pending snippet exits zero with its notice because the id exists; an unknown id exits non-zero.

## See Also

- [project/instrukt/spec/domain/docs](../../domain/docs.md)
- [project/instrukt/spec/feature/docs/progressive-retrieval](progressive-retrieval.md)
