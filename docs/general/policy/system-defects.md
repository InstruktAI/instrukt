---
description: Every agent assesses erratic behaviour of the system it works through and, when a current contract appears broken and an occurrence is in hand, reports it through `instrukt bugs report`, facing its questions before filing.
visibility: 'public'
---

# System Defects — Policy

## Rules

- **You assess the system you work through.** That system is instrukt, the
  agent runtime you run in and the platform they run on. When it behaves
  erratically — a command fails or answers wrongly, a session or message goes
  missing, an effect happens twice or not at all — judge whether a current
  contract states otherwise. A person's or a company's own service, reached
  through `instrukt mcp`, is not this system: its errors are relayed to the
  person it belongs to.
- **A broken contract with an occurrence in hand goes through the front
  door.** Run `instrukt bugs report '<description>'`. It files nothing and
  prints the questions to face first: bug or change, evidence, duplication,
  ordering, owning project. Answer each against the sources it names; file
  with `--approved` only once all are faced, into the project that owns the
  defect on this computer. An occurrence is an address a reader can open that
  shows the behaviour in real use: a log record, a command's output, a
  delivered message, a session transcript (`sdlc/concept/defect-evidence`).
  Without one, the behaviour stays an observation in your session and nothing
  is filed.
- **The answers decide what happens.** Where no current contract states
  otherwise, it is not a bug: file a change only when the behaviour is wanted,
  and otherwise nothing. Where the defect is already reported, being fixed or
  a triaged known issue, add your occurrence to that work or file nothing.
  Where work in flight owns it, route it there.
- **A worker reports to the session that governs it.** When your procedure
  routes what you notice outside your job to a governing session, report the
  observation there; that session decides it and files
  (`sdlc/policy/todo-creation-discipline`).
- **Reporting widens nothing.** Carry on with your own work. A report neither
  authorizes you to repair the defect outside your assignment nor to work
  around it by changing the system.
- **The report stays out of the conversation you serve.** A person you serve
  hears only what belongs to their world: the part of their request that
  cannot be done right now. The defect, the report and instrukt's internals
  are the operator's.

## Rationale

The agents working through instrukt meet its failures first and most often.
A failure only an agent saw is lost when its session ends, and the next
agent meets it again with no record. One front door that puts the same
questions to every filer keeps the record to defects a contract states
otherwise, filed once, in the project that can repair them. The command
checks that the questions were faced, never that the answers are true:
judging them belongs to the filer and to preparation, which retires a claim
whose evidence does not hold.

A person a company's agent serves cannot act on instrukt's internals, so
telling them about a defect costs them attention and gives them nothing to do.

## Scope

Every agent in every session, including a company's agent in a
conversation, for behaviour of instrukt, its runtime and its platform.

## Enforcement

- Preparation judges each filed report by the evidence it carries and
  retires one whose evidence fails (`sdlc/procedure/lifecycle/prepare/diagnose`).
- Reviewers and peers treat a contract break witnessed with an occurrence in
  hand and left unreported, a report filed without facing the questions, and
  a report surfaced to a person a company's agent serves as findings against
  the turn.

## Exceptions

None.

## See Also

- [sdlc/concept/defect-evidence](../../domain/sdlc/concept/defect-evidence.md)
- [sdlc/policy/todo-creation-discipline](../../domain/sdlc/policy/todo-creation-discipline.md)
- [general/procedure/company-assistance](../procedure/company-assistance.md)
