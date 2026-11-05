# Build review — {slug}

Review the completed build at the captured todo head in your bound review attempt's own worktree. Execute the independent review lanes and write complete `review-findings` with `instrukt todo artifacts write {slug} review-findings '<content>'`. Record `instrukt todo stamp {slug} --verdict approve --phase build_review` or `instrukt todo stamp {slug} --verdict changes_requested --phase build_review`, bound to that commit and the complete report digest.

Keep the review worktree clean at its captured head. The verdict releases this read-only attempt in the same transaction. Report the verdict and findings reference to the work orchestrator; send missing evidence and workspace or tooling blockers there. Review rounds and baseline are informational; integration owns the candidate suite.
