---
description: 'File instrukt/runtime/platform defects via `instrukt bugs report` (answer questions first; keep out of served chats).'
activation_trigger: 'When instrukt, runtime, or platform violates its contract with an occurrence in hand.'
visibility: public
---

# Bug Reporting — Procedure

## Goal

A defect of the system you work through reaches the project that can repair
it, once, with its evidence, and nothing that is not a defect is filed. That
system is instrukt, the agent runtime you run in and the platform they run
on. A person's or a company's own service, reached through `instrukt mcp`, is
not this system: its errors are relayed to the person it belongs to.

## Preconditions

- Behaviour of that system went against what it should do: a command failed
  or answered wrongly, a session or message went missing, an effect happened
  twice or not at all.
- You hold an occurrence: an address a reader can open that shows the
  behaviour in real use — a command and its output, a log record, a delivered
  message, a session transcript (`sdlc/concept/defect-evidence`). A filer
  whose own contract admits a dangerous path in its place
  (`sdlc/policy/todo-creation-discipline`) may hold that instead. Without
  either, the behaviour stays an observation in your session and nothing is
  filed.

## Steps

1. **Bug or wish.** Name the current contract the behaviour breaks: a
   snippet, or the declared outcome of the surface that misbehaved. A failure
   shows that something happened, not that it is wrong. Where no current
   contract states otherwise it is not a bug: file a change only when the
   behaviour is wanted, and otherwise nothing.
2. **Report or fix in context.** When your procedure routes what you notice
   outside your job to a governing session, report the observation there and
   stop: that session decides it and files
   (`sdlc/policy/todo-creation-discipline`). Otherwise, when the repair is
   documentation-only or a bounded code change of about five minutes, make it
   yourself at once in a short-lived one-off worktree and file nothing
   (`sdlc/policy/todo-creation-discipline`, Fix bounded findings in context).
   A company's agent never holds repair authority: for it the answer is always
   to report.
3. **Open the front door.** Run `instrukt bugs report '<description>'`. It
   files nothing and prints the questions to face: fix in context or file, bug or change,
   validity, evidence, duplication, ordering, owning project.
   Answer each against the sources it names:
   - **Duplication.** In the owning project, read `instrukt todo list
     --project <slug>`, `instrukt bugs list --project <slug>` and `instrukt
     bugs suppressions --project <slug>`. A defect already reported, being
     fixed or triaged as a known issue gets your occurrence added to that work,
     or nothing.
   - **Ordering.** Work in flight that owns the defect, or that it depends on,
     receives it instead of a new filing.
   - **Owning project.** The project that owns the defect on this computer,
     named with `--project <slug>`. A symptom that stops once the observing
     project's own configuration matches an instrukt default is that project's
     to fix, there.
4. **File.** Only once every question is faced, run the approved form the
   command printed, with `--approved` and the owning project. Put the
   observations and their addresses first; keep any causal account apart
   from them, with what remains unknown, or the statement that the cause is
   not established.
5. **Carry on.** Return to your own work. The report authorizes neither a
   repair outside your assignment nor a workaround that changes the system.
6. **Keep the report out of the conversation you serve.** A person you serve
   hears only what belongs to their world, in plain words: "I can't do that
   part right now." The defect, the report and instrukt's internals are the
   operator's.

## Outputs

- One filed bug in the owning project, an occurrence added to existing work,
  an observation handed to your governing session, or nothing filed with the
  reason in your session.

## Recovery

- **The front door refuses the project.** Name one the refusal lists, or file
  nothing when none of them owns the defect.
- **The front door itself fails.** Keep the occurrence and the refusal in your
  session, and carry on with your work; the failure is itself an occurrence
  for the operator.

## See Also

- [general/procedure/company-assistance](company-assistance.md)
