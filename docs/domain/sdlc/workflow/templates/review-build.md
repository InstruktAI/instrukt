# Build review — {slug}

Owed: independent review of the completed build, complete `review-findings`, and a verdict bound to the current reviewed commit and report digest. Record the report with `instrukt todo artifacts write {slug} review-findings '<content>'`, then `instrukt todo stamp {slug} --verdict approve --phase build_review` or `instrukt todo stamp {slug} --verdict changes_requested --phase build_review`. Review counts and the baseline are informational. Add no suite execution.

Before starting, read the role binding with `instrukt todo show {slug}` and observe its recorded session through `instrukt sessions alive <id>`. Continue an available session with `instrukt sessions send <id> '<brief>'`; `run` is the start branch, not an implicit reuse operation. The runner records the returned runtime-native id per todo and role; preserve that binding. Use the adapter's supported selection against the procedure's dispatch order or the global order; when selection is unsupported, report the runtime's project defaults. An ambiguous start remains pending for recovery, never a second dispatch. A reply or ended turn does not establish completion.

Review rounds are counted from verdict stamps, including verdicts in a reused session. Re-evaluation alone is not a round; no admission write is required.
