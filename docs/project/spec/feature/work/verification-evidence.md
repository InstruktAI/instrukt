---
description: Raw evidence of live verification runs is kept outside the repository and outside what the work record reflects, found by reference for a bounded time.
delivered_by:
  - verification-stamp-and-evidence-store
---
# Verification Evidence — Spec

## What it is

Raw verification evidence is what a live verification run captures from the
real surface it drives: recordings, screenshots, transcripts and observed
state. The work record carries the sanitized `verification-plan` and
`verification-findings` artifacts and, where a run retained its raw evidence,
a reference to it. The evidence itself stays outside Git and outside
reflection.

## Canonical fields

### What the record carries

Todo artifacts are the published work evidence and reflect to the issue
tracker (`project/instrukt/spec/feature/work/github-record`). Raw verification
evidence is never an artifact, is never committed to a repository and is never
reflected.

Where a run retained raw evidence, `verification-findings` carries a reference
to it. A reference identifies the run; it is not a machine-local path. The
verification disposition recorded for the candidate binds that report
(`project/instrukt/spec/feature/work/stamps`).

### CI-hosted runs

For a CI-hosted run the reference is the whole of what the record keeps.
Access to the evidence and how long it is kept are decided by the home that
holds it. Whether such a run retains raw evidence, and in which authorized
home, is decided by the publication boundary of
`sdlc/policy/live-verification`.

### Local evidence store

A local run that keeps raw evidence beyond its own teardown keeps it in the
computer's verification evidence store. The store belongs to the instrukt home
and lies outside every repository and worktree: evidence in it makes no
worktree dirty and reaches no commit. Evidence is held per project, todo and
run, and is found on that computer from the reference its findings carry.

The store and its entries are readable and writable only by the owner of the
instrukt home. Every path instrukt reads, writes or removes in the store stays
within the store's canonical location, so a home reached through a symbolic
link keeps its store. A project, todo or run identifier, a path inside an
entry or a symbolic link in the store that would resolve outside that location
is refused before anything is read, written or removed through it
(`sdlc/policy/security-awareness`).

The store is derived state (`sdlc/policy/state-storage`): its source is the
candidate commit and the verification run, and losing an entry costs running
that verification again. Nothing of another nature is kept in it.

instrukt removes an entry once its retention has lapsed. No agent deletes
evidence from the store, and removing an entry changes no stamp, artifact or
reference.

Keeping evidence in the store is optional for a run. Evidence a run does not
keep stays in that run's own temporary location and ends with the run.

## Use cases

### UC-VE1: Inspect what a local verification run observed after the run ended

```gherkin
Given a developer's candidate was verified by a local run that kept its raw evidence
When the developer follows the findings' evidence reference on that computer while retention lasts
Then the run's raw evidence is readable there
And none of it is in the repository or in the todo's reflected record
When the evidence's retention has lapsed
Then instrukt has removed it without an agent deleting it
```

## Known caveats

- A local run's evidence is readable only on the computer that ran it; it does
  not follow a todo whose claim moves to another computer.
- instrukt neither stores nor fetches the evidence of a CI-hosted run.

## See Also

- [project/instrukt/spec/feature/work/stamps](stamps.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md)
- [project/instrukt/spec/domain/work](../../domain/work.md)
