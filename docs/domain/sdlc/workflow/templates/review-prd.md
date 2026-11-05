# PRD review — {slug}

Owed: an independent review of the current `prd`, a complete `prd-review-findings` report and a digest-bound verdict. Review counts are informational.
Write the report with `instrukt todo artifacts write {slug} prd-review-findings '<content>'` before `instrukt todo stamp {slug} --verdict approve --phase prd_review` or `instrukt todo stamp {slug} --verdict needs_work --phase prd_review`.
The verdict must bind the current target and report; zero unresolved blocking findings earns approval.
Judge any carry of earlier product approval in the report; the claimant records carry only from this current independent conclusion, retaining the earlier attribution.

Before starting, read the role binding with `instrukt todo show {slug}` and observe its recorded session through `instrukt sessions alive <id>`. Continue an available session with `instrukt sessions send <id> '<brief>'`; `run` is the start branch, not an implicit reuse operation. The runner records the returned runtime-native id per todo and role; preserve that binding. Choose the first available entry in the procedure's dispatch order or the global order. A reply or ended turn does not establish completion.

Review rounds are counted from verdict stamps, including verdicts in a reused session. Re-evaluation alone is not a round; no admission write is required.
