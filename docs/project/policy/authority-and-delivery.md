---
description: This repository's documentation is the authority for everything instrukt covers, its work is done directly with the human in the loop, and TeleClaude is read as the predecessor.
---

# Authority and Delivery — Policy

## Rules

### This repository's documentation leads

- instrukt replaces TeleClaude. For everything instrukt covers, the
  documentation in this repository states what is true and what is wanted:
  `docs/general/`, `docs/domain/` and `docs/project/`.
- Ground in this repository's documentation and code, and read its
  documentation as files in the working tree. On a computer that runs
  TeleClaude, an agent CLI's user-scope instruction file is written by
  TeleClaude and directs agents to TeleClaude's documentation and lifecycle;
  for work in this repository, this policy takes its place on both.
- TeleClaude's documentation and code show how a capability worked before
  instrukt. Where they differ from this repository's documentation, this
  repository's documentation is the answer.

### Work is done directly, with the human in the loop

- The human directs the work on this repository in conversation and iterates
  on it until it is complete. It is not delivered through todos.
- Documentation leads the code: a change in behavior is written into the
  owning snippet first, and the code follows in the same line of work.
- A snippet states the agreed contract as it reads once the work is complete.
  It carries no pending carrier: no planned marker, no companion file and no
  `delivered_by`, which belong to delivery through todos. While work is under
  way a snippet may state behavior the code does not deliver yet; that
  difference is work to finish.
- An agreement with the human that changes what a snippet establishes is
  written into that snippet when it is reached, by the agent in that
  conversation.

### TeleClaude's tools are for reading

- TeleClaude's tools may be used to read its documentation, its session
  history and its state. Work for this repository changes nothing through
  them.

## Rationale

- An agent answers from the documentation it grounds in. Where the
  instructions it starts with name TeleClaude as the source of truth, it
  answers with the names and behavior instrukt has changed, and a decision
  made for instrukt reaches no one.
- An agreement that lives only in a conversation or a working note is absent
  from what the next session loads, so that session reasons without it. The
  owning snippet is the one place every later agent reads.
- Pending carriers keep a live contract from running ahead of code while many
  todos change the same documentation. This repository has one line of work,
  directed by the human until it is complete, so there is nothing for a
  carrier to coordinate and it only postpones the agreed contract.
- TeleClaude holds the behavior and history instrukt is built from. Reading it
  prevents reinvention; changing state through it would run this repository's
  work under the lifecycle it replaces.

## Scope

Every agent working on this repository. It governs how this repository itself
is worked on: the todo workflow and the pending carriers that `docs/general/`
and `docs/domain/sdlc/` describe stay what instrukt provides to the projects
that adopt it.

## Enforcement

- Reviewers of this repository's changes apply these rules.

## Exceptions

- None.
