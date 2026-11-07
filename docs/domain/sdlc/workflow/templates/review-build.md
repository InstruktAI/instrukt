# Build review — {slug}

Confirm your native identity is bound to the reserved role attempt before workspace work or record writes. If binding is not yet visible, report that prerequisite to your parent and retain this attempt for continuation.

Review the completed build at the captured todo head in the shared todo worktree under your bound review attempt. Execute the independent review lanes and write complete `review-findings` with `instrukt todo artifacts write {slug} review-findings '<content>'`. Record `instrukt todo stamp {slug} --verdict approve --phase build_review` or `instrukt todo stamp {slug} --verdict changes_requested --phase build_review`, bound to that commit and the complete report digest.

Keep the shared worktree clean at its captured head. The verdict releases this read-only attempt in the same transaction. Report the verdict and findings reference to the work orchestrator; include missing evidence and workspace or tooling blockers in that report. Review rounds and baseline are informational; integration owns the candidate suite.

Close your turn with the verdict, findings reference and bound review attempt. The runtime delivers your report and ending to your parent.
