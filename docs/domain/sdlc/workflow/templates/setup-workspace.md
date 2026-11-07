# Complete shared workspace setup — {slug}

Owed: complete setup in the todo worktree already recorded for your orchestrator.

Work in that same worktree on the todo branch under your bound shared setup attempt. Capture the original base before synchronization. Fetch and merge accepted upstream main, make the opening commit through `instrukt repo commit`, run the project's configured `prepare` action when declared, and publish the todo branch. Record the original base and approved plan with `instrukt todo stamp {slug} --build-started`, and write `quality-checklist` through `instrukt todo artifacts write {slug} quality-checklist '<content>'`, including `## Build Gates (Builder)`, Build and Review sections and project-specific gates.

Verify the shared tree is clean, then record `instrukt todo stamp {slug} --role-complete`. Completion releases your attempt atomically. Report its head, setup outcomes and evidence references to your parent. Resolve synchronization within this attempt; report a blocker with preserved work and native status when it cannot continue.
