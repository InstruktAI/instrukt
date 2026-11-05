---
eval: true
description: "Convert a task todo's raw input into a durable, repeatable task-plan
  and request configured authorization. Non-SDLC: task routing within prepare,
  with no PRD, architecture, implementation plan or reviewer verdict."
role: task-preparer
parameters:
  - name: slug
    required: true
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: med
---

# Prepare Task — Procedure

## Required reads

@~/.instrukt/docs/general/principle/continuity.md
@~/.instrukt/docs/sdlc/spec/todo-kind-task.md

## Goal

Turn a `kind=task` todo's raw `input` into a `task-plan` — the durable,
authorized instruction set the task executes on every run — and then request
the configured authorization. This procedure does not execute the task or
create other todos. It is the task route of `instrukt todo prepare {slug}`.
A task is non-SDLC: no PRD, architecture, implementation-plan, or reviewer
verdict applies.

## Preconditions

- The todo is `kind=task` and the current prepare action calls for task-plan
  drafting, with an `input` artifact and no `task-plan` yet (or a refinement
  pass on an existing one).
- The session runs in the project on `main`, with no worktree.
- The `input` is the versioned intent record. Its current position-based
  intent check is required; a new version invalidates the earlier check.

## Steps

Use `instrukt <command> -h` for individual commands.

0. **Audit the handoff.** Read `instrukt todo show {slug}` and the `input`.
   Apply the entry audit in `sdlc/policy/todo-creation-discipline`. Take a
   current intent check as given; the first non-author recipient of unchecked
   input reads its originating conversation through the sessions facade and
   records `instrukt todo stamp {slug} --intent-checked`.
   If the originating conversation is permanently unavailable, return the intent to the
   human in the responsible thread; a new `input` version from that conversation becomes
   the checkable basis.
   Establish the requested action boundary, result, recipient, and timing from
   what the `input` carries, including its logically necessary implications and
   existing authoritative constraints. If another reasonable execution could
   satisfy everything the input carries while omitting or varying a proposed
   operational or business limit, that limit is unspecified. Keep it out of requested scope or present it explicitly for the
   human to decide during authorization.

1. **Read the `input`.** `instrukt todo artifacts read {slug} input`. Read it as
   intent to interpret, not a spec to obey literally. When Step 0 applies, use
   the verified action boundary rather than treating the entire agent-authored
   input as the human's request.

2. **Reject if the intent touches the repo.** Judge the input against the
   boundary in `sdlc/spec/todo-kind-task`. If fulfilling it
   requires modifying a repository file or making a commit, do not draft a
   `task-plan`: report to the human that the todo is misclassified — name the
   repo-touching work implied — and recommend `instrukt todo update {slug}
--kind <change|feature|bug>`. Stop here.

3. **Check for an obvious duplicate.** Scan `instrukt todo list` for a plausible
   match — the shallow, bounded check `sdlc/spec/todo-kind-task`
   requires. If one turns up, surface it to the human before drafting rather
   than proceeding; do not attempt deeper reconciliation.

4. **Interpret it into a `task-plan`.** Author the plan as the durable, repeatable
   contract reused on every firing:
   - an intent line ("I read your request as …");
   - the operations the run performs;
   - the result handoff the human asked for, if any — what to produce and where it
     goes — an output artifact or destination supported by the configured
     tooling. Questions needing the human are asked in the run session's
     thread; the plan claims no external messaging capability;
   - whether the run reports its `log`.
     The plan is the _what_; the run agent decides the _how_ with its own tooling.

5. **Write the `task-plan`.** `instrukt todo artifacts write {slug} task-plan "…"`.

6. **Request configured authorization.** Call `instrukt todo prepare {slug}`
   after writing the plan and follow its rendered authorization instructions. Human approval is
   the default: the responsible session presents the `task-plan` and asks in
   its thread. A peer gate uses a separate peer session to check the boundary,
   result, recipient and timing against `input`, with no invented limits,
   no repository work, the shallow duplicate check completed, and the intent
   line, operations, result handoff and log present. There is no separate
   reviewer verdict. A peer escalates money, public statements in the human's
   name, granting access and legal commitments to the human in the thread.
   Record authorization and its human or peer attribution through
   `instrukt todo stamp {slug} --task-approved --by human|peer [--session <runtime:id>]`;
   GitHub approvals arrive by pull. Authorization binds `task_plan.digest` through
   `task_approved.target_digest` and retains `task_approved.by`; writing the plan never authorizes it.

   When scheduling is requested, retain the input's timing on the task record
   using `instrukt todo update {slug} --schedule <value>`: an ISO-8601 instant
   or `cron:<minute hour day month weekday>`. The runner owns due-time and
   missed-run decisions; scheduling never substitutes for authorization.

7. **Hand off to the runner.** Once the record carries the current `task-plan`
   and authorization stamp, call prepare for the remaining obligation. The
   runner dispatches `sdlc/procedure/lifecycle/task/run-task` when the task is
   authorized, not held, eligible under pickup policy, and due with its
   dependency gate satisfied.

## Outputs

- A `task-plan` artifact (durable, repeatable instruction set).
- A configured authorization request and evidence-bound human or peer stamp;
  the intent record remains versioned separately from the task plan.

## Recovery

- **Input too sparse to interpret.** Surface the gap and ask one direct question;
  do not invent scope into the plan.
- **The `input` does not support the claimed request.** Do not draft from the
  agent-authored additions. Surface the mismatch and wait for a human-owned
  action boundary.
- **Intent requires repo modification.** Refuse per Step 2; do not draft a
  `task-plan`.
- **Plausible duplicate found.** Refuse per Step 3; surface it to the human
  rather than guessing whether it is truly redundant.
- **Refinement.** When the human revises the intent, rewrite the `task-plan` and
  request configured authorization again. A changed plan invalidates its prior
  approval binding; a new input version also invalidates the intent check.

## Discipline

You draft and request approval; you never execute the task and never authorize it
yourself. The `task-plan` is the durable contract a run agent will
execute repeatedly across firings — write it so it
stands on its own across many future firings, not as a one-time script. Sliding from
"classify this as task" into drafting a plan for repo-touching or duplicate work
without checking Steps 2 and 3 first, or from "draft the plan" into "do the work,"
are the failure modes here.
