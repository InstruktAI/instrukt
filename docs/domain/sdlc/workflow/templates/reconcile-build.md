# Build plan reconciliation — {slug}

Confirm your native identity is bound to the reserved role attempt before workspace work or record writes. If binding is not yet visible, report that prerequisite to your parent and retain this attempt for continuation.

The build was bound to {build.plan_digest}; the current plan is {implementation_plan.digest}. In your bound builder attempt in the shared todo worktree, read `instrukt todo artifacts diff {slug} implementation-plan {build.plan_digest} {implementation_plan.digest}`. Preserve what serves the current plan, undo what it invalidates and complete the remaining tasks. Before resuming implementation, re-record `instrukt todo stamp {slug} --build-started` against the current approved plan, preserving the original base.

Record dispositions, commit the reconciled work, complete the checklist and record `instrukt todo stamp {slug} --build-complete` against the clean committed head. Report that head and completion evidence to the work orchestrator after atomic completion and attempt release. Send plan defects and workspace or tooling blockers there with the evidence and preserved work location.
