---
description: Encrypted nightly backup and guarded restore of an instrukt home to provider-neutral S3-compatible object storage — dynamic SQLite consistency snapshots, client-side age encryption, per-machine namespacing, staged verification and retention pruning, local standing and bookkeeping, and safe restore.
---
# Home Backup — Spec

## What it is

Home backup protects a computer's irreplaceable instrukt home (`~/.instrukt`, or
an explicit `INSTRUKT_HOME`) by publishing consistent, client-side encrypted
snapshots to provider-neutral S3-compatible object storage and providing a
guarded restore command. It is self-contained inside instrukt with no coupling
to external deployment tooling.

An operator runs backups on demand or through an owned daily OS timer converged
by `project/instrukt/spec/feature/runner/module-installation`, inspects remote
generations for this computer, sees local backup standing through
`project/instrukt/spec/feature/publication/instrukt-status`, receives anomaly
notification when backups fail or go stale through
`project/instrukt/spec/feature/runner/anomaly-notification`, and restores into
a stopped home or an alternate directory with full database integrity
verification.

Home backup protects the host's instrukt home as a whole; exporting and
importing one isolated company container remains owned by
`project/instrukt/spec/feature/company/company-containers`.

## Canonical fields

### Commands

| Command | Contract |
| --- | --- |
| `instrukt backup run` | Convert any legacy company home settings forward through configuration, snapshot irreplaceable home state consistently, compress and encrypt client-side, publish through staged verification into this machine's namespace, prune excess generations for this machine, and record local bookkeeping. |
| `instrukt backup list` | List backup generations in the configured bucket for this machine, newest first, distinguishing validated from unvalidated generations. |
| `instrukt backup restore [--from <key>] [--machine <id>] [--into <dir>] [--identity <path>] [--yes]` | Download, decrypt, safely extract, verify every restored SQLite database, and restore a backup generation under overwrite, disk-headroom, and live-home guards. |

Every subcommand accepts `-h` and prints its own contract without executing.
Global configuration and entry validation are owned by
`project/instrukt/spec/feature/configuration/layered-configuration`; when home
backup is not configured or is withheld due to a configuration finding, backup
commands refuse with that finding and its configuration door
(`sdlc/principle/design-fundamentals`, Fail Fast;
`general/principle/trust-and-guardrails`). Command capability admission and
refusal of company-assigned sessions are owned by
`project/instrukt/spec/feature/configuration/roles-and-capabilities`.

### Snapshot scope

Home backup preserves irreplaceable machine and company-home state and omits
regenerable, ephemeral, and git-governed trees.

| Category | Included |
| --- | --- |
| Machine SQLite stores | Every `*.db` and `*.sqlite` database discovered dynamically under `state/` (including the work/runner state database, retained history, review stores, requested-review store, connector read-back store, and any future stores added there). |
| Machine configuration, secrets and durable state | Global `instrukt.yaml`, `agents.yaml`, `secrets.env`, `state/machine-id`, `state/eval-fingerprint/`, `state/agents-deployment.json`, `state/known-issues/` (excluding lock files), and `state/company-generation/` when present. |
| Global research and exploration notes | Durable research notes under `git/` (such as `index.md`, `history.md`, and repo-scoped non-checkout notes) and `explore/` (cross-project exploration reports), included when present and skipped cleanly when absent. |
| Company home local state | Under `companies/<name>/`, with older machine-owned settings requiring explicit `instrukt install` first (`project/instrukt/spec/feature/configuration/owned-files`): `company.yaml`, `secrets.env`, local state under `.instrukt/`, and every `*.db` and `*.sqlite` database discovered dynamically under `.instrukt/` (such as `knowledge-proposals.db`). Collection never migrates settings. |

Excluded unconditionally from every snapshot:

- `cache/` (explicitly including `cache/git/`, the fixed cache for re-cloneable
  external repositories) and any raw git working-tree checkout directory
  containing `.git` under `git/`;
- published and rendered documentation rebuilt by sync (`docs/`, `rendered/`,
  `state/index.yaml`, `state/publication/`);
- regenerable and ephemeral state (`state/blueprint/`, `state/configuration/`,
  `state/installation/`, `state/hook-runs/`, `state/redis/`,
  `state/consumer-locks/`, `state/runner-hosts.json`, `state/backup-state.json`,
  `state/backup-staleness-notified.json`, `state/.backup-staging-*`,
  `state/.backup-restore-staging-*`, `state/.backup-restore-rollback-*`,
  `locks/`, all `*.lock` files, and `companies/<name>/.instrukt/schemas/`);
- live SQLite sidecars (`*-wal`, `*-shm`, `*-journal`), stray recovery copies
  (`*.corrupt.*`, `*.pre-*`, `*.recovered-*`), and ad-hoc local `backups/`;
- git-tracked company repository working trees under `companies/<name>/`
  (`.git/`, tracked repository files, and `projects/` checkouts), installed
  tool binaries and virtual environments, and disposable runtime state outside
  the home.

**Path confinement:** Snapshot collection confines every collected file and
SQLite store to real paths inside its owning root (the instrukt home for
machine state and research, and each company's own folder under
`companies/<name>/` for company files). Escaping or unresolvable symlinks are
never dereferenced and are refused fail-closed; symlinks are never archived as
symlink entries (`sdlc/policy/security-awareness`).

### Consistency, encryption and scale

- **Consistent SQLite snapshotting:** Every discovered `*.db` and `*.sqlite`
  file under `state/` and `companies/<name>/.instrukt/` is copied into
  temporary staging through SQLite's online backup mechanism rather than
  copying raw live files or WAL sidecars, then checked with
  `PRAGMA quick_check` before archiving. Any snapshot or consistency failure
  aborts publication before uploading or pruning clean generations, records the
  failed run locally, and exits non-zero.
- **Client-side encryption and non-disclosure:** The staged snapshot is
  compressed and encrypted client-side using `age` to the configured public
  recipient (`project/instrukt/spec/feature/configuration/layered-configuration`),
  so unattended runs hold no private decryption key on disk
  (`sdlc/policy/security-awareness`). Secret material and credentials are never
  written to logs, terminal output, restore diagnostics, persisted local
  failure summaries, or anomaly notifications
  (`project/instrukt/spec/feature/configuration/owned-files`).
- **Multi-GB streaming and headroom:** Archiving, compression, encryption,
  upload, and server-side promotion stream without whole-file memory buffering
  so multi-GB homes succeed across any archive size. Before staging, backup
  checks available disk space against the shared-machine headroom requirement
  (`general/policy/shared-machine-resources`) and refuses without writing
  staging data if headroom would be violated; temporary staging files are
  cleaned up on both success and failure.
- **Outside-service availability:** As one-shot CLI invocations, calls to the
  S3-compatible endpoint use the established S3 client directly with bounded
  retries and ordinary error handling (`sdlc/policy/substrate-anchoring`); an
  outage leaves existing remote generations intact, cleans staging on a
  best-effort basis, and records the failed run locally.

### Offsite publication, verification and retention

- **Per-machine namespace:** Generations live under
  `<prefix>/<machine-id>/instrukt-home.<timestamp>.tar.<ext>.age` (or
  `<machine-id>/instrukt-home.<timestamp>.tar.<ext>.age` when `prefix` is
  omitted) using this computer's `state/machine-id`, so multiple machines share
  one bucket without colliding.
- **Staged verify-promote-validate protocol:**
  1. Upload the encrypted archive to `_staging/<key>` using SigV4 with plain
     fixed-length or multipart upload framing without `aws-chunked`
     streaming-trailer checksums.
  2. Read object metadata (`HEAD`) on `_staging/<key>` and verify an exact byte
     size match against the local encrypted archive. On mismatch or upload
     error, perform best-effort deletion of `_staging/<key>` and refuse to
     publish.
  3. Promote `_staging/<key>` to `<key>` via in-region server-side copy,
     handling both single-object copy and multipart server-side copy across the
     provider's single-object copy size limit.
  4. Write the `_validated/<key>` marker object, then delete `_staging/<key>`.
- **Per-machine retention pruning and stale orphan cleanup:** After writing
  `_validated/<key>`, prune only this machine's namespace to the configured
  generation retention count
  (`project/instrukt/spec/feature/configuration/layered-configuration`).
  Unvalidated generations are evicted before validated generations, oldest
  first within each group; pruning a validated generation removes its paired
  `_validated/<key>` marker as well. Also remove stale orphaned objects under
  `_staging/<namespace>/` and abort stale incomplete multipart uploads across
  both `_staging/<namespace>/` and `<namespace>/` aged past runner's
  orphan-cleanup window, scoped strictly to this computer's `machine-id`.
  Generations, staging objects, and
  multipart uploads belonging to other machine IDs are never touched.
- **Post-validation failure accounting:** Once `_validated/<key>` is written,
  local bookkeeping still advances the recorded last successful timestamp and
  published generation even if subsequent `_staging/<key>` deletion, retention
  pruning, or orphan cleanup fails, while recording the post-validation error
  (redacted), setting the run outcome to `failed`, invoking `runner.notify`,
  and exiting non-zero.

### Guarded restore

- **Target selection:** Without `--from`, restore selects the newest
  `_validated/` generation for this computer's `machine-id` (or `--machine <id>`
  when recovering another machine's backup). `--from <key>` selects an explicit
  object key and permits recovering a named explicit or unvalidated generation.
- **Identity input:** `--identity <path>` takes the path to an `age` private
  identity file (or `-` to read from standard input); raw secret keys are never
  accepted as command-line arguments (`sdlc/policy/security-awareness`). When
  `--identity` is omitted, restore prompts with echo off if attached to a
  terminal and refuses fail-closed naming `--identity <path>` in
  non-interactive sessions.
- **Headroom, decryption, safe extraction and full integrity check:** Before
  staging the download and extracted tree, restore checks available disk space
  across every filesystem written during staging and swap against the
  shared-machine headroom requirement
  (`general/policy/shared-machine-resources`) and refuses without staging if
  headroom would be violated, re-checking headroom while extracting. Restore
  stages on the same filesystem as each target subtree (handling mountpoints
  and symlinked `INSTRUKT_HOME` or `state/` roots without cross-device rename
  failures), downloads the encrypted archive into owner-only temporary staging,
  decrypts it using the supplied private identity, extracts entries while
  rejecting path traversal, absolute paths, and every non-regular/non-directory
  archive entry type (symlinks, hardlinks, devices, FIFOs, sockets), and runs
  `PRAGMA integrity_check` on every restored `*.db` and `*.sqlite` database
  before modifying the destination. Any decryption, extraction, or integrity
  failure aborts without touching the destination. Temporary restore staging is
  cleaned up unconditionally on success, failure, and declined confirmation.
- **Live-home lock guard and destination symlink confinement:** Canonicalizing
  `--into <dir>` against active `INSTRUKT_HOME` ensures aliases resolving to
  the live home trigger the live-home lock guard. Before opening or creating
  any live-home lock file or mutating the destination, restore enforces
  leaf-inclusive component-by-component symlink confinement below the resolved
  destination root, resolved `state/` directory, and each company folder. When
  restoring into the active instrukt home, restore verifies confinement on
  every fixed and dynamically discovered live-home lock path, then acquires
  non-blocking exclusive locks across all active home, company, and
  known-issues locks and holds them across the entire swap; if any lock is
  held, restore refuses before modifying the destination, naming the holder
  where known and naming the doors: stop instrukt services first, or pass
  `--into <dir>` to extract beside the live home for inspection or selective
  recovery (`general/principle/trust-and-guardrails`). After locks are held and
  before any swap mutation or permission change, restore enforces the same
  leaf-inclusive symlink confinement on every restored destination path and its
  SQLite sidecars.
- **Rollback-preserving swap:** During swap, replaced destination files are
  moved into same-filesystem rollback staging before renaming staged files into
  place, and pre-existing SQLite sidecars (`-wal`, `-shm`, `-journal`) are
  moved into that same-filesystem rollback staging after each main database's
  atomic rename succeeds (never unlinking sidecars mid-swap). On any mid-swap
  failure, restore rolls back replaced files, databases, and matching sidecars
  from rollback staging; if rollback fails, restore reports the partial state
  and directs the operator to re-run `instrukt backup restore --from <key>` (or
  `--into <dir>`). Rollback staging and moved sidecars are deleted only once
  the entire swap commits.
- **Alternate target (`--into <dir>`):** When `<dir>` resolves outside the
  active instrukt home, extracts the verified backup contents into `<dir>`
  without acquiring or modifying the live instrukt home.
- **Restored schemas:** After a live-home swap, restore snapshots restored
  older configuration and databases before explicitly migrating them, using
  the install migration boundary. Runner, installation, and backup locks stay
  held until migration finishes; per-store and document locks are acquired by
  their migration owners. An alternate target keeps the archived schemas for
  inspection. Restore requires services to be stopped and does not start them.
- **Overwrite confirmation:** Restore prompts unconditionally for confirmation
  before modifying the destination unless `--yes` is supplied. When
  `--identity -` consumes standard input on an interactive terminal,
  confirmation reads from the controlling terminal; non-interactive sessions
  without `--yes` fail closed.

### Local bookkeeping, standing query, scheduling and alerting

- **Dedicated local bookkeeping:** Each backup run atomically records its
  outcome (`first_attempt_at` preserved from the first attempt,
  `last_attempt_at`, `last_outcome`, `last_success_at`, `last_generation`, and
  redacted `last_error`) in a dedicated local state file outside `instrukt.db`
  and `writer.lock` (`sdlc/policy/state-storage`), preserving the last
  successful timestamp and generation when a subsequent attempt fails. A
  failure writing bookkeeping on a failed run never suppresses `runner.notify`
  and surfaces alongside the primary failure.
- **Backup standing query (`runner → publication`):** Runner owns the
  local-only, lock-free backup standing query consumed by
  `project/instrukt/spec/feature/publication/instrukt-status`. Evaluating
  global backup configuration, local bookkeeping, and runner's staleness
  threshold without contacting remote storage, it returns runner's standing
  judgment (`unconfigured`, `never_run`, `ok`, `failed`, or `stale`),
  `admitted`, `stale`, `last_attempt_at`, `last_success_at`, `last_generation`,
  redacted `last_error`, or a structured `(path, reason)` unreadable-state error
  if bookkeeping cannot be read.
- **Failure and staleness alerting:** Runner owns the staleness threshold past
  the daily schedule. On backup failure, `instrukt backup run` raises an
  anomaly directly through
  `project/instrukt/spec/feature/runner/anomaly-notification` on every host
  (including hosts with neither `instrukt tick` nor `instrukt run`). Staleness
  is evaluated from local bookkeeping without network calls: it surfaces on
  every host via `instrukt status` and `instrukt events status`, and actively
  notifies through `runner.notify` whenever `instrukt tick` or `instrukt run`
  runs on the host, clearing once a subsequent backup succeeds.
- **Daily scheduling:** Convergence, inspection, and withdrawal of the owned
  daily OS timer that invokes `instrukt backup run` without starting an agent
  session belong to `project/instrukt/spec/feature/runner/module-installation`.

## Use cases

### UC-HB1: Back up an instrukt home consistently and encrypted offsite

```gherkin
Given an operator has configured encrypted offsite home backup on their computer
And the computer's instrukt home holds active SQLite stores, machine settings and secrets, durable operational state, global research and exploration notes, a company home with local state beside its git-tracked checkout, and excluded caches, locks, raw git checkouts and stray recovery files
When the operator runs a home backup
Then every SQLite store in machine state and company homes is snapshotted consistently and verified before archiving
And the archive includes the irreplaceable machine state, research notes and company home local state confined to each owning root while omitting caches, locks, live database sidecars, recovery copies, published docs, raw git checkouts and git-tracked company repository trees
And the encrypted archive is verified in remote staging, promoted to this computer's namespace, marked validated and cleaned from staging
And the archive cannot be read without the matching private decryption key
And local backup bookkeeping records the completed generation
But when a database fails consistency verification or the staged remote upload fails verification
Then nothing is published to the computer's namespace, existing validated generations remain intact, staged remote data is cleaned up and the failure is recorded locally
```

### UC-HB2: List this computer's backup generations

```gherkin
Given an operator's configured backup bucket holds generations for two different computers
And this computer's namespace holds both validated and unvalidated generations
When the operator lists backups on this computer
Then the listing shows this computer's generations newest first, distinguishing validated from unvalidated
And generations belonging to the other computer are omitted
```

### UC-HB3: Restore a home backup with integrity, headroom and live-home guards

```gherkin
Given an operator has a validated encrypted home backup in remote storage and holds its matching private decryption identity file
When the operator restores into a stopped home or an alternate target directory and confirms overwrite
Then disk headroom is verified before staging, the archive is decrypted and safely extracted, every restored SQLite database passes full integrity verification before the destination is modified, and temporary staging is cleaned up
And restoring into an alternate directory leaves the live home untouched
But when the operator attempts to restore into a live home while instrukt services hold active locks
Then restore refuses before modifying the destination and names the options to stop services or restore into an alternate directory
And when the archive fails decryption, contains unsafe paths, fails database integrity verification or the operator declines overwrite confirmation
Then restore aborts without modifying the destination and cleans up temporary staging
```

### UC-HB5: Prune excess backup generations per machine without disturbing other machines

```gherkin
Given an operator's configured backup bucket holds generations for two different computers
And this computer's namespace already holds both validated and unvalidated generations at the configured retention count
When the operator runs a home backup on this computer
Then only the configured number of generations remain for this computer, evicting unvalidated generations before validated ones
And evicted generations are no longer selectable as validated backups
And the other computer's generations are untouched
```

## Known caveats

- Decryption requires the operator's private `age` identity kept outside the
  backed-up home; unattended nightly runs never store the private key on disk,
  so losing the private identity makes remote archives unrecoverable.
- Git-tracked company repositories and project checkouts are intentionally
  excluded from home backup and are recovered from their git remotes; any
  uncommitted local work in those checkouts is not included.
- On a backup-only host where neither `instrukt tick` nor `instrukt run` is
  installed, a failed `instrukt backup run` still invokes `runner.notify`,
  while a missed timer that never started surfaces as stale on local inspection
  (`instrukt status`, `instrukt events status`) rather than via push
  notification.
- Restoring a home backup onto a fresh machine recovers irreplaceable state and
  configuration; regenerable publication outputs, editor schemas, blueprint
  graphs, and owned OS units are rebuilt afterward with `instrukt sync` and
  `instrukt install`.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/owned-files](../configuration/owned-files.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/company/company-containers](../company/company-containers.md)
- [project/instrukt/spec/feature/publication/instrukt-status](../publication/instrukt-status.md)
