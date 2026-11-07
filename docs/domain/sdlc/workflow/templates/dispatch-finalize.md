# Finalize shared candidate — {slug}

Owed: synchronize, publish and complete the candidate in the shared todo worktree.

In your bound shared finalizer attempt, read current approval and processed deferrals. Merge accepted upstream, resolve conflicts preserving both intents, and publish with `git push origin HEAD`. Verify the clean shared head, local todo branch and remote head match. Record `instrukt todo stamp {slug} --role-complete`; this releases the attempt atomically. Report the candidate, completion and any changed content or review evidence to your parent for independent review routing.

After your parent confirms current approval and release of all workspace roles, recheck that matching candidate and record `instrukt todo stamp {slug} --finalize-ready` against your retained binding. This record-only write appends `workflow.finalized` with readiness for that exact commit. Report evidence or a mismatch. Workspace changes require a new attempt. A synchronization blocker is reported with preserved shared work; integration owns the candidate suite and promotion.
