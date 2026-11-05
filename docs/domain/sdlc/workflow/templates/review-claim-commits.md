# Claim commit reviews — {slug}

Owed: independent commit review for every SHA in {claim_commits.unreviewed}. Follow the commit-review procedure; `instrukt repo stamp <sha…> --verdict approve --report <file>` records the verdict and report digest as git notes, or use `needs_work` for findings. For a commit owing no review, its author records `instrukt repo stamp <sha> --verdict exempt --reason <text>`. A `needs_work` note remains in the unsettled set; only `approve` or `exempt` permits release. A count or a reply is insufficient.

Report the resulting artifact and stamp evidence to the dispatching orchestrator. Send missing evidence, scope questions and workspace or tooling blockers there with their concrete evidence.
