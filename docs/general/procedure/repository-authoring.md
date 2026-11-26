---
description: 'Author and commit in a git repository when no brief or assignment names the workspace: place the work under the commit policy, commit through instrukt, route review and hand the head to the integrator.'
activation_trigger: 'Use before the first write to a git repository — an edit, branch, worktree or commit — when no brief or assignment names the workspace to write in.'
visibility: public
---

# Repository Authoring — Procedure

## Required reads

- [general/policy/commits](../policy/commits.md)

## Goal

Place, commit and hand off repository work exactly as `general/policy/commits`
gives it, so no agent invents a worktree, branch name or location.

## Preconditions

- The repository is registered and its canonical branch can be fetched.
- Authority for the change exists: an explicit execution request, or a human
  conversation that released it (`sdlc/policy/todo-creation-discipline`).

## Steps

1. **Place the work.** Work on a todo belongs to its orchestrator and the
   workers it dispatches in the todo's worktree; you hold neither role without
   an assignment saying so, so your work is ad hoc. That includes pending
   documentation you write ahead for a todo. Work ad hoc in a one-off worktree
   from the freshly fetched canonical branch, on a branch prefixed with the
   change's Commitizen type and never a todo's branch
   (`general/policy/commits`).
2. **Author and verify.** Make the change there and run the checks the
   repository's own instructions name, `instrukt docs validate` for snippets
   included.
3. **Commit through instrukt.** Commit with `instrukt repo commit`, declaring
   human involvement, and with `--todo <slug>` when the change carries pending
   documentation for that todo.
4. **Route review.** Decide by the routing rule of `general/policy/commits`
   whether independent review is owed; when it is, have the exact head
   reviewed before handoff.
5. **Hand off and clean up.** Hand the reviewed head to the integrator, the
   only writer of main. Once it is merged, remove the one-off worktree and its
   branch.

## Outputs

- A reviewed, committed head on a one-off branch, merged onto main by the
  integrator, with its one-off worktree removed.

## Recovery

- A refused commit or review stays with this session: correct the change and
  commit again on the same branch.
- Work begun in a todo's worktree or on its branch without that todo's
  assignment moves to a one-off worktree before any further commit.
