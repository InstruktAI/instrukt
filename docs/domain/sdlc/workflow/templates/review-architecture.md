# ARCHITECTURE review — {slug}

Owed: an independent review of the current `architecture`, a complete `architecture-review-findings` report and a digest-bound verdict. Review counts are informational.
Write the report with `instrukt todo artifacts write {slug} architecture-review-findings '<content>'` before `instrukt todo stamp {slug} --verdict approve --phase architecture_review` or `instrukt todo stamp {slug} --verdict needs_work --phase architecture_review`.
The verdict must bind the current target and report; zero unresolved blocking findings earns approval.

Review rounds are counted from verdict stamps, including verdicts in a reused session. Re-evaluation alone is not a round; no admission write is required.

Report the resulting artifact and stamp evidence to the dispatching orchestrator. Send missing evidence, scope questions and workspace or tooling blockers there with their concrete evidence.
