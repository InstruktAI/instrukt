# Workspace setup — {slug}

Work in your bound setup attempt's returned worktree at its captured todo head. Return its path and remain available for setup follow-ups from the work orchestrator. The orchestrator runs upstream synchronization, the opening commit, configured project preparation and first publication there.

After those outcomes succeed, confirm the worktree is clean, record `instrukt todo stamp {slug} --role-complete`, and report its committed head and completion evidence to the orchestrator. Resolve synchronization failures within this outstanding attempt when you can. Otherwise report the reason, committed role head, path and unfinished workspace state to the orchestrator for `instrukt todo handoff {slug} --failed --reason <text>`. Preserve the work there for a recovery builder admitted after confirmed failed release.
