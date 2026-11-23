---
description: Free disk space is shared by every process on the machine; whoever creates a checkout or build output keeps it bounded, removes it when its work ends, and keeps a floor of headroom before starting a build.
visibility: 'public'
---

# Shared Machine Resources — Policy

## Rules

- **Disk headroom belongs to the whole machine.** Every agent, the human's
  applications and their virtual machines write to the same volume. An agent
  keeps at least 20 GiB free while it works, whichever project it serves.
- **Whoever creates a checkout owns its output.** A session that creates a
  worktree, clone, review copy, temporary `HOME` or scratch directory removes
  its regenerable output when that checkout's work ends: Rust `target/`,
  `node_modules`, `.venv`, build and test caches, temporary homes. It removes
  the checkout itself once nothing needs it. Committed work and anything not
  regenerable stay.
- **Delegation carries the duty.** A session that delegates work which creates
  checkouts names that cleanup in the brief, and when a round of delegated work
  ends, removes the regenerable output its finished delegates left behind.
<!-- planned:reduce-worktree-disk-usage -->
- **Waiting work checkouts drop their build output.** When a todo's work
  checkout transitions into an idle wait — after its candidate is handed to
  integration, or when the orchestrator parks or holds the todo after build
  output was created there — the session that owns the regenerable build output
  created there removes that output at the transition. The clean committed
  tree, the todo branch and any published head remain intact; if work later
  resumes in that checkout, the next build regenerates what it needs. Active
  builds and checkouts awaiting review before handoff are not covered by this
  rule.
<!-- /planned:reduce-worktree-disk-usage -->
- **Measure before building in a new place.** Before a build, install or full
  test run in a checkout that has no build output yet, check the volume's free
  space. With less than 20 GiB free, clean up output this session owns first;
  if that does not restore 20 GiB, stop and report the free space and the
  largest regenerable directories rather than building.
- **Reuse before duplicating.** Work that needs only evidence a build already
  produced, such as a hook run or CI result for the same commit, cites that
  evidence instead of rebuilding in another checkout.
- **Clean what you own.** A session removes only output it or its delegates
  created, never another live session's checkout or build output. Output of a
  session that has ended is the delegating session's to remove.

## Rationale

Each isolated checkout builds its own full output, and parallel work
multiplies it: a few concurrent builders and reviewers produce enough
regenerable output to fill a volume. When the volume fills, writes fail in
unrelated applications and their virtual machines stop. That harm lands
outside the session that caused it, where restoring the files repairs nothing.
Output is regenerable, so removing it costs only a rebuild; keeping it costs
everyone on the machine.

<!-- planned:reduce-worktree-disk-usage -->
Once a todo's work checkout hands off to integration or parks on an external
wait, it sits idle for the full duration of downstream integration or the wait
with no live build inside it. Keeping its regenerable output turns disk usage
into a function of every todo in flight rather than active builds, risking the
machine's headroom floor. Dropping that output at the transition leaves
committed work untouched and pays at most one rebuild if work returns.
<!-- /planned:reduce-worktree-disk-usage -->

## Scope

All agents, all sessions, all projects, on every machine they run on.

## Enforcement

<!-- planned-change:reduce-worktree-disk-usage -->
Reviewers treat a finished checkout left with its build output as a
process miss.
<!-- change:reduce-worktree-disk-usage -->
Reviewers treat a checkout left with build output that the rules above require
removed as a process miss.
<!-- /planned-change:reduce-worktree-disk-usage -->

## Exceptions

None.
