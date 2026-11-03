# Integration handoff pending — {slug}

The finalized commit {finalize.sha} matches the clean worktree, local todo branch and remote branch, with current independent approval and processed deferrals. The `--finalize-ready` stamp records the handoff and appends `workflow.finalized` idempotently for project/todo/SHA; confirm that evidence, then release. Integration belongs to the standing integrator.
Inspection confirms the recorded stamp and event; it writes neither and releases no claim.

The release and cleanup steps below become owed only after the handoff and event-log append recorded by `--finalize-ready` are confirmed.

Release the execution claim with `instrukt todo update {slug} --unclaim` and confirm its successful outcome and record evidence through `instrukt todo show {slug}`. A refused release stays pending. `instrukt todo update {slug} --release` removes a dispatch hold, not the execution claim. Retain the working resources on return to preparation.
After release is positively confirmed (`claim.held` is false), use the todo's `sessions.bound` bindings as the assignment child set and end those children by their recorded ids with `instrukt sessions end <id>`; the configured runtime must offer `end`. End the lane session with `instrukt sessions end {runtime.session_id}` only when its remaining obligations are complete. An unfinished human conversation keeps its session and gives up its claim. Missing cleanup capability remains explicit.
