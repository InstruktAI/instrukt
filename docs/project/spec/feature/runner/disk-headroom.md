---
description: The disk-headroom standing job — keeps the volume every process on the computer shares above its free-space headroom, warning once per crossing and, below the floor, removing regenerable build output from idle linked checkouts only, failing closed on anything it cannot read.
visibility: public
delivered_by: [instrukt-first-build]
---
# Disk Headroom — Spec

## What it is

Every agent, the developer's applications and their virtual machines write to
one volume. When it fills, writes fail far from whatever filled it. The
disk-headroom guard keeps that volume above its headroom: it warns the
developer when free space runs low and, below a hard floor, removes
regenerable build output from linked checkouts no running process uses, until
the headroom is back. It never touches a primary clone or a nested repository,
and it removes nothing it could not fully read.

The guard is a standing job of the runner. A standing job is work the runner
performs itself on its wake-ups, on its own cadence: no todo, no task record,
no authorization and no agent session. Its directive counterpart is
`general/policy/shared-machine-resources`.

## Canonical fields

### Cadence and measurement

The guard runs on runner wake-ups (`instrukt tick`, `instrukt run`), at most
once every five minutes, under the runner lock. It runs whatever modules are
switched on; `runner.disk_headroom: off` switches it off.

Each run measures the free space of the volume holding the developer's home
directory and classifies it:

| Free space | Level |
| --- | --- |
| at or above `warn` | `ok` |
| below `warn`, at or above `floor` | `warn` |
| below `floor` | `floor` |

`runner.disk_headroom` (global only) is `off` or `{warn: <size>, floor:
<size>}`, sizes in GiB (`30GiB`). The defaults are `warn: 30GiB` and `floor:
10GiB`. A floor that is not positive and below the warn level is a validation
finding.

### Checkouts the guard may clean

The guard cleans only linked checkouts: git worktrees whose `.git` is a file.
It finds them as:

- the linked worktrees registered by each project checkout recorded in
  publication state;
- under each directory in `runner.disk_headroom.roots` (global only, default
  none): the linked worktrees registered by every primary clone found there,
  and every linked checkout found there directly.

A checkout is excluded when it is a primary clone (its `.git` is a
directory), contains a primary clone, lies outside every place it was found
through, or is reached through a symbolic link. The search descends into no
symbolic link and stops at each primary clone it finds.

### Regenerable output

Inside an eligible checkout, a candidate is the outermost directory of one of
these kinds:

| Kind | Recognized by |
| --- | --- |
| Rust build output | a directory named `target` holding `CACHEDIR.TAG`, `.rustc_info.json` or `debug/.fingerprint` |
| Node dependencies | a directory named `node_modules` |
| Python environment | a directory named `.venv` holding `pyvenv.cfg` |

A nested checkout or repository inside the checkout is its own owner: its tree
is not searched. A directory is a candidate only when git confirms the checkout
tracks nothing inside it, and only when its whole tree was read and holds no
checkout or repository. Its size is the space its files occupy, each
hard-linked file counted once.

### Warn

On entering `warn` from `ok`, or at the first run that observes `warn`, the
guard reports once: free space, the warn level, and the largest candidates
(up to ten) with their sizes and checkouts. It removes nothing. Further runs at
`warn` report nothing until the level has returned to `ok`.

### Floor

At `floor` the guard reclaims. It takes candidates largest first. Before each
removal it re-measures free space, stopping once it is back at the warn level,
and re-reads which paths this user's running processes hold. A candidate whose
checkout holds a working directory, executable or open file of such a process
is skipped as in use. A removal that fails is recorded and the next candidate
is taken.

A floor run reports: free space before and after, what it removed, what it
skipped as in use, what it could not remove, and, when free space is still
below the floor, that headroom could not be restored. A floor run that removes
nothing after an earlier floor run reports nothing.

### Failing closed

The guard acts only on what it read:

- Free space that cannot be measured: no action; the run is reported once
  until a measurement succeeds.
- A place or checkout that cannot be read, or whose worktrees git cannot list:
  none of its checkouts are candidates.
- Tracking that git cannot confirm, or a candidate tree not fully read: not a
  candidate.
- Any of this user's running processes whose working directory, executable or
  open files cannot all be read: every checkout counts as in use, so nothing is
  removed, and the report says why.

### Reports and state

A report goes through the runner's anomaly notification
(`project/instrukt/spec/feature/runner/anomaly-notification`). The
notification states the level and free space, and names `instrukt ticks
status`; the report itself stays in the runner's state. `instrukt ticks
status` shows the guard's last run: level, free space and its last report. The
runner's state keeps the last level, which makes a report fire once per
crossing. Each removal is logged with its path and size.

### Use cases

#### UC-HR1: A developer learns the disk is running low before builds fail

```gherkin
  Given a developer's computer has anomaly notification available
  And free space on the shared volume drops below the warn level
  When the runner next wakes up
  Then the developer is told once how much space is free
  And can see the largest regenerable directories in their agents' checkouts
  And nothing is removed
  And while space stays below the warn level they are not told again
```

#### UC-HR2: Space is reclaimed at the floor from idle checkouts only

```gherkin
  Given free space on the shared volume drops below the floor
  And agents' linked checkouts hold build output, some in checkouts a running process uses
  When the runner next wakes up
  Then build output is removed, largest first, from checkouts no running process uses, until free space is back at the warn level
  And the output of checkouts in use is left in place
  And primary clones, nested repositories and anything the checkout tracks are never touched
  And the developer is told what was removed, what was skipped, and whether headroom was restored
```

#### UC-HR3: The guard removes nothing it cannot verify

```gherkin
  Given free space on the shared volume is below the floor
  And the guard cannot read what one of the developer's running processes holds open
  When the runner next wakes up
  Then nothing is removed
  And the developer is told that headroom could not be reclaimed and why
```

#### UC-HR4: A developer switches the guard off

```gherkin
  Given a developer has switched the disk-headroom guard off for their computer
  When free space on the shared volume drops below the floor
  Then the runner neither reports nor removes anything for disk headroom
```

## Known caveats

- Only processes running as the developer's own user are inspected; a process
  of another user that holds a path in a checkout does not keep it in use.
- With anomaly notification unavailable, reports reach the developer only
  through `instrukt ticks status` and the log.

## See Also

- project/instrukt/spec/domain/runner
- project/instrukt/spec/feature/runner/task-timing — wake-ups the guard runs on
- project/instrukt/spec/feature/runner/anomaly-notification — where reports go
- general/policy/shared-machine-resources — what agents do to keep the headroom
