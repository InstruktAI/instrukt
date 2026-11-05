# Prepared; release — {slug}

Before release, confirm the current `input` has its independent intent check, the configured PRD or task approval is satisfied by the recorded approver, and each claim commit has `approve` or `exempt`. A `needs_work` note, stale input check or ineligible approver keeps release owed. All selected preparation obligations must be current, including claim commit reviews. The runner owns subsequent execution dispatch.

Release the execution claim with `instrukt todo update {slug} --unclaim` and confirm its successful outcome and record evidence through `instrukt todo show {slug}`. A refused release stays pending. `instrukt todo update {slug} --release` removes a dispatch hold, not the execution claim. Retain the working resources on return to preparation.
After release is positively confirmed (`claim.held` is false), record the handoff and any remaining obligations. Preserve conversations needed for later rounds or human input. The runtime owns session ending and cleanup. An unfinished human conversation keeps its context and gives up its claim.
