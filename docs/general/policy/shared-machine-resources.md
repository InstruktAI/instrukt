---
description: Disk space, CPU and memory are shared by every process on the machine; whoever creates a checkout, build output or ephemeral process keeps it bounded, removes or reaps it when its work ends, and keeps a floor of headroom before starting a build.
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
- **instrukt reclaims todo worktrees.** A todo's worktree is outside the
  removal duties this policy gives a session: instrukt returns the disk it
  holds, and no session deletes its build output, or the worktree itself, with
  a shell command. The runner removes the regenerable output the project
  declares in its `instrukt.yaml` from a todo worktree that is idle — its
  candidate handed to integration, its todo parked or held, or its work gone
  back to preparation — and removes a delivered todo's worktree and work
  branch. A worktree whose todo is still being built, reviewed or fixed keeps
  its output for the role that works there next. The clean committed tree, the
  todo branch and any published head remain intact; if work later resumes in
  that worktree, the next build regenerates what it needs. Where the project
  declares no regenerable output, nothing is removed until the delivered
  worktree goes as a whole.
- **Measure before building in a new place.** Before a build, install or full
  test run in a checkout that has no build output yet, check the volume's free
  space. With less than 20 GiB free, clean up output this session owns first;
  if that does not restore 20 GiB, stop and report the free space and the
  largest regenerable directories rather than building. In a todo's worktree
  that cleanup is `instrukt repo reclaim`: the session that writes there runs
  it whenever the volume falls below the floor, before a build or during one,
  and it removes that worktree's declared regenerable output and nothing else.
- **Reuse before duplicating.** Work that needs only evidence a build already
  produced, such as a hook run or CI result for the same commit, cites that
  evidence instead of rebuilding in another checkout.
- **Clean what you own.** A session removes only output it or its delegates
  created, never another live session's checkout or build output. Output of a
  session that has ended is the delegating session's to remove.
- **Reap ephemeral processes you spawn.** Any temporary server, container,
  background load or reproduction process a session starts is spawned in a
  tracked group and torn down in a guaranteed-cleanup path (`trap` / `finally`
  / process-group kill) the instant its evidence is captured — never left as a
  detached child leaking CPU or memory across the host.

## Rationale

Each isolated checkout builds its own full output, and parallel work
multiplies it: a few concurrent builders and reviewers produce enough
regenerable output to fill a volume. When the volume fills, writes fail in
unrelated applications and their virtual machines stop. That harm lands
outside the session that caused it, where restoring the files repairs nothing.
Output is regenerable, so removing it costs only a rebuild; keeping it costs
everyone on the machine. The same holds for ephemeral processes: a detached
reproduction server, container or load generator left running after a session
moves on steals CPU and memory from every other agent and application on the
host until someone hunts it down.

A todo's worktree sits idle for the whole of a wait — on integration, on a
person, or on preparation — with no live build inside it. Keeping its
regenerable output turns disk usage into a function of every todo in flight
rather than active builds, risking the machine's headroom floor. Removing that
output leaves committed work untouched and pays at most one rebuild if work
returns. instrukt removes it because the work record already holds every fact
the removal turns on — which worktree is which todo's, whether a role is
working in it, whether the todo is delivered — so the removal needs no agent's
judgment. A deletion command issued from an agent's shell stops at the agent
runtime's permission prompt and reaches a person as a line they cannot judge.

## Scope

All agents, all sessions, all projects, on every machine they run on.

## Enforcement

Reviewers treat a checkout left with build output that the rules above require
removed, or an ephemeral process left orphaned after its work ends, as a process
miss.

## Exceptions

None.
