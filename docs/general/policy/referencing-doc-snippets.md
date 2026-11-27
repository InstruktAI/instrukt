---
description: 'Name snippets by id, never path, in Required reads, See Also, and briefs; references run upward only.'
visibility: 'public'
---

# Referencing Doc Snippets — Policy

## Rules

- A reference names a snippet by its id — `general/policy/leadership`,
  `sdlc/spec/domain-spec`, `project/<slug>/spec/domain/<context>` — in
  `## Required reads`, in See Also and in prose citations alike. It never names
  a file path or a home location; the CLI resolves the id. A code reference
  stays a repository-relative path.
- A `## Required reads` entry may name a domain by its bare name (`sdlc`). It
  stands for that domain's `expanded` set with its closure, the content
  `instrukt docs get --domain <x>` renders in full.
- Required reads are static declarations in documentation snippets. A runtime-generated brief, formatter message, or peer message that needs a snippet read names its id and, when needed, the `instrukt docs get <id>` invocation, so the receiving session performs the read under its own required-read discipline.
- A required read names only what the snippet needs beyond the closure of its
  other required reads. A read another read already implies is a validation
  finding that names the read implying it.
- References run upward only, from a specific body toward the generic one. In
  Required reads, a bare domain read included, and in See Also, a `general`
  snippet names only `general`; a domain snippet names only its own domain
  and `general`, never another domain; a project snippet names its own
  project, `general` and any domain, never another project. An `index.yaml`
  names only its own position's snippets
  (`general/spec/snippet-authoring-schema`).
- A published snippet never references project material, neither as a Required
  read nor as a citation. It states the general contract. How one project realizes
  that contract lives in the project's own snippets, which agents working
  there load themselves.
- A placeholder pattern that names the reader's own project — `project/<slug>/…`
  where `<slug>` is the reader's project — is a pattern, not a reference, and may
  appear in any snippet.
- Third-party sources must appear under **See also** only (soft references).
  They must not appear in Required reads.

### Inline citations

- A rule another snippet owns is cited in prose, never restated: a one-clause
  gist followed by the owner's backticked id — _orient the human before a
  consequential move (`general/policy/leadership`)_. The gist carries the rule's
  force where the reader acts; the id routes the reader to the single place that
  states it in full, with its reason. A Gherkin scenario is a rule for this
  purpose: a predicate another spec owns is not re-authored as a use case under
  a different actor or surface; the citing spec carries its one-sentence
  contribution and the owner's id.
- A bare backticked id is a pointer, not a citation. Use it where the reader
  needs to know where something lives, not what it requires.
- Cite by id only — never by file path, never by section anchor. A rule that can
  only be located with an anchor lives in a snippet too large to own it.
- A citation never paraphrases the owner beyond its gist. When the citing text
  needs more of the rule than one clause carries, the reader needs the owner:
  make it a Required read, or narrow the citing text so one clause suffices.
- A gist names what the rule requires where the citing text acts, in the
  owner's own terms — never the owner's internal structure (a list, a section,
  a heading). A gist that names structure breaks silently when the owner is
  reorganised.

## Rationale

- Required reads are the single enforced mechanism for mandatory reading declared by a static artifact. Runtime instructions name the required snippet without rendering it so the receiving session controls the read once.
- An id is the same from every location and every computer, while a path names
  one machine's layout. Resolving ids in the CLI keeps snippets free of home
  paths and lets the same text be read anywhere.
- A domain's `expanded` set is what everything in that domain needs; naming the
  domain keeps the dependency on that set as it changes, where copying its ids
  would go stale.
- A closure is additive: a reader receives everything every read implies. A
  read that another read already implies adds no knowledge and hides which
  read the snippet actually depends on.
- A published snippet is read in every project, and a project snippet only in its
  own. A citation into project material is a dead end for every other reader, and
  it ties the general contract to one project's implementation. The dependency runs
  from the specific toward the general, never the reverse.
- Each body stands on its own. `general` is read on every computer, and a
  domain is published as one unit by one owner and adopted independently of
  its siblings: a reference from `general` into a domain, or from one domain
  into another, either pulls that domain's doctrine into readers that never
  chose it or dead-ends where it is not adopted. Copying `general` into a
  domain instead would be duplication, so a domain points up at `general`.
- Soft references prevent third-party docs from being auto-included in agent context.
- A rule stated in two snippets is two rules to a reader: each wording is read
  as possibly distinct, and the copies drift until they contradict. The gist
  keeps the emphasis a repetition was reaching for; the id keeps one owner.
- A citation is only reviewed when something touches it. Reviewing every
  citation in a changed snippet, and every citer of a changed owner, catches
  drift from both directions without opening review to unrelated content.

## Scope

- Applies to all AI agents, all repositories, and all documentation references in static artifacts or runtime messages.
- Static-declaration rules apply to every doc snippet that requires prerequisites and to the `index.yaml` entries that name snippets.

## Enforcement

- Static Required reads must appear in a `Required reads` section immediately after the H1 title.
- Validation reports a Required read that is not a known id or domain, a read another read's closure already implies, and a Required read or See Also entry against the reference direction.
- Soft references must appear in a `See also` section at the bottom.
- Reviewers treat a restated rule whose owner is another snippet as a finding and
  replace it with a citation.
- A change to a snippet extends reference review to the whole of that snippet:
  its author checks every citation in it before committing, and the reviewer
  checks them again, touched or not. Fixes are limited to the citations
  themselves — never the snippet's own content.
- A change to what a snippet establishes extends reference review to the
  snippets that cite it: its author finds them by its id, and the reviewer
  checks that each citation still holds against the changed owner.
- Whether a backticked id in prose is a citation, an example, or a placeholder
  is a reading judgment, so no scanner checks inline citations
  (`general/principle/intelligence-boundaries`).

## Exceptions

- None.
