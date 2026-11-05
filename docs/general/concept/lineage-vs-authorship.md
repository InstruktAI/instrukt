---
description: 'Identity on a record is the authority the work ran under, never who did the work. Attribution questions are answered from creation records and session evidence, not account identity.'
visibility: 'public'
---

# Lineage vs Authorship — Concept

## What

Every durable record — a git commit, a todo issue, an approval — carries
identity: an author field, a GitHub account. That identity names the
**authority the work ran under**, not the party that did the work. This is an
autonomous system: agents do the work, and the human's identity threads through it as lineage because the
tools run under their account. A commit authored under the human's name was
almost always written by an agent. A todo issue opened under the human's
GitHub account was, more often than not, filed by an agent operating under
that account.

Attribution questions — "who created this?", "who wrote that?", "did you do
this?" — are answered from the evidence built for them: a todo's creation
record, a commit's `Session:` trailer, session transcripts. Account identity is
never that evidence.

Identity on a record never gates a mechanism. Work with no resolvable session
commits exactly as any other work does: its commit carries no `Session:`
trailer, which is its true state, not a missing value. A mechanism neither
refuses, holds nor degrades on that absence, and never guesses an identity in
its place.

## Why

Agents repeatedly commit the same misreading: they inspect git history or an
author field, find the human's name, and conclude the human authored the work
— then assert it back to the human, who is forced to defend against their own
audit trail. The misreading inverts reality precisely where truth matters
most: in an autonomous system, the human is _involved_ in work agents perform; the human is not its
author, its driver, or its origin by default. Reading lineage as authorship
misattributes agent work to humans, corrupts audits, and erodes the human's
trust in every conclusion the agent draws downstream. The rule exists so no
agent ever again treats an authority thread as a signature.

A mechanism that reads identity as a functional input breaks for exactly the
work that carries none it expects, such as a session no runtime resolves.
Guessing an identity instead attributes the work to someone who never directed
it. Both failures come from treating an authority record as a precondition.

## See Also

- [general/policy/human-involvement-recording](../policy/human-involvement-recording.md) — which facts carry a human-involvement record, and why the declaration is never derived from identity.
