# PLAN review — {slug}

Owed: an independent review of the current `implementation-plan`, a complete `plan-review-findings` report and a digest-bound verdict. Review counts are informational.
Write the report with `instrukt todo artifacts write {slug} plan-review-findings '<content>'` before `instrukt todo stamp {slug} --verdict approve --phase plan_review` or `instrukt todo stamp {slug} --verdict needs_work --phase plan_review`.
The verdict must bind the current target and report; zero unresolved blocking findings earns approval.

Review rounds are counted from verdict stamps, including verdicts in a reused session. Re-evaluation alone is not a round; attempt reservation and native binding record dispatch facts without counting a round.

Report the resulting artifact and stamp evidence to the dispatching orchestrator. Include missing evidence, scope questions and workspace or tooling blockers in your report with their concrete evidence.
