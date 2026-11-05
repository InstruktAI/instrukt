# Return to preparation — {slug}

The plan review has been invalidated by a kickback or substantive drift. Read the record to identify its cause; the cleared marker alone does not prove which occurred. A new kickback is the work orchestrator's judgment, recorded with `instrukt todo stamp {slug} --kickback --reason '<why the plan cannot be built>'`. This appends the reason to input, invalidates the plan and resets finalization. Retain the worktree, branches and commits.

Release the execution claim with `instrukt todo update {slug} --unclaim` and confirm its successful outcome and record evidence through `instrukt todo show {slug}`. A refused release stays pending. `instrukt todo update {slug} --release` removes a dispatch hold, not the execution claim. Retain the working resources on return to preparation.
After release is positively confirmed (`claim.held` is false), record the handoff and any remaining obligations. Preserve conversations needed for later rounds or human input. The runtime owns session ending and cleanup. An unfinished human conversation keeps its context and gives up its claim.
