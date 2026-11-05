# Workspace setup — {slug}

This setup-only run is bound to todo `{slug}` as a setup-role attempt in its own runtime-created worktree from the current todo head. Return the recorded path to the orchestrator and remain available for setup follow-ups in that active attempt.

The orchestrator runs the ordered setup effects in this worktree: upstream synchronization, opening commit through `instrukt repo commit`, project preparation when declared, then first branch publication. Report failures and keep setup outstanding until those obligations succeed. Implementation belongs to the later builder attempt.

After setup succeeds, confirm the worktree is clean and report its committed head. The orchestrator fast-forwards the todo branch to that stamped head and confirms the handoff and setup-attempt release before recording build start, creating the checklist and admitting the builder at the advanced todo head. Complete this setup attempt with `instrukt todo stamp {slug} --role-complete`.
