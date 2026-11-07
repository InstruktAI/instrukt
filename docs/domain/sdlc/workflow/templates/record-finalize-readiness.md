# Record candidate readiness — {slug}

Work orchestrator: confirm the latest finalizer completion and release, current independent approval, reviewer release, and equality of the clean shared worktree, todo branch and published head. If content or review evidence changed during finalization, obtain independent review at that current shared head first. Send the retained finalizer a record-only follow-up through the configured child-message instruction to record `instrukt todo stamp {slug} --finalize-ready` and report its evidence.

Further workspace changes require a new shared finalizer attempt and completion before readiness is reconsidered.
