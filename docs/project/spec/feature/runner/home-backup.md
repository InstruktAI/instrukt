---
description: Encrypted nightly backup and guarded restore of an instrukt home to S3-compatible object storage — dynamic SQLite consistency snapshots, client-side age encryption, per-machine namespacing, staged verification and retention pruning, local bookkeeping, and safe restore.
delivered_by: [instrukt-home-backup]
---
# Home Backup — Spec

## What it is

Home backup protects a computer's irreplaceable instrukt home (`~/.instrukt`, or
an explicit `INSTRUKT_HOME`) by publishing consistent, client-side encrypted
snapshots to S3-compatible object storage (such as DigitalOcean Spaces) and
providing a guarded restore command. It is self-contained inside instrukt with
no coupling to external deployment tooling.

An operator runs backups on demand or through an owned daily OS timer converged
by `project/instrukt/spec/feature/runner/module-installation`, inspects remote
generations per machine, sees local backup standing in
`project/instrukt/spec/feature/publication/instrukt-status`, receives anomaly
notification when backups fail or go stale through
`project/instrukt/spec/feature/runner/anomaly-notification`, and restores into
a stopped home or an alternate directory with full database integrity
verification.

Home backup protects the host's instrukt home as a whole; exporting and
importing one isolated company container remains owned by
`project/instrukt/spec/feature/runner/company-containers`.

## Canonical fields

### Commands

| Command | Contract |
| --- | --- |
| `instrukt backup run` | Snapshot irreplaceable home state consistently, compress and encrypt client-side, publish through staged verification into this machine's S3 namespace, prune excess generations for this machine, and record local bookkeeping. |
| `instrukt backup list [--machine <id>]` | List backup generations in the configured S3 bucket for this machine (or `<id>`), newest first, distinguishing validated from unvalidated generations. |
| `instrukt backup restore [--from <key>] [--machine <id>] [--into <dir>] [--identity <age-key>] [--yes]` | Download, decrypt, safely extract, verify every restored SQLite database, and restore a backup generation under overwrite and live-home guards. |

Every subcommand accepts `-h` and prints its own contract without executing.
Global configuration and entry validation are owned by
`project/instrukt/spec/feature/configuration/layered-configuration`; when home
backup is not configured or fails entry validation, backup commands refuse with
that finding and its configuration door
(`project/instrukt/policy/checks-and-guards`).

### Snapshot scope

Home backup preserves irreplaceable machine and company-home state and omits
regenerable, ephemeral, and git-governed trees.

| Category | Included |
| --- | --- |
| Machine SQLite stores | Every `*.db` and `*.sqlite` database discovered dynamically under `state/` (including the work/runner state database, retained history, review stores, requested-review store, connector read-back store, and any future stores added there). |
| Machine configuration, secrets and durable state | Global `instrukt.yaml`, `agents.yaml`, `secrets.env`, `state/machine-id`, `state/eval-fingerprint/`, `state/agents-deployment.json`, and `state/runner-hosts.json`. |
| Global research and exploration notes | `git/` (ingested repository research such as `index.md`, `history.md`, and repo-scoped notes) and `explore/` (cross-project exploration reports), included when present and skipped cleanly when absent. |
| Company home local state | Under `companies/<name>/`: `company.yaml` (and untracked legacy pre-migration `instrukt.yaml` declaring `company`), `secrets.env`, local state under `.instrukt/`, and every `*.db` and `*.sqlite` database discovered dynamically under `.instrukt/` (such as `knowledge-proposals.db`). |

Excluded unconditionally from every snapshot:

- `cache/` (explicitly including `cache/git/`, the fixed cache for re-cloneable
  external repositories);
- published and rendered documentation rebuilt by sync (`docs/`, `rendered/`,
  `state/index.yaml`, `state/publication/`);
- regenerable and ephemeral state (`state/blueprint/`, `state/configuration/`,
  `state/installation/`, `state/hook-runs/`, `state/redis/`,
  `state/consumer-locks/`, `locks/`, all `*.lock` files);
- live SQLite sidecars (`*-wal`, `*-shm`, `*-journal`), stray recovery copies
  (`*.corrupt.*`, `*.pre-*`, `*.recovered-*`), and ad-hoc local `backups/`;
- git-tracked company repository working trees under `companies/<name>/`
  (`.git/`, tracked repository files, and `projects/` checkouts), installed
  tool binaries and virtual environments, and disposable runtime state outside
  the home.

### Consistency, encryption and scale

- **Consistent SQLite snapshotting:** Every discovered `*.db` and `*.sqlite`
  file under `state/` and `companies/<name>/.instrukt/` is copied into
  temporary staging through SQLite's online backup mechanism rather than
  copying raw live files or WAL sidecars, then checked with
  `PRAGMA quick_check` before archiving. Any snapshot or consistency failure
  aborts publication before uploading or pruning clean generations, records the
  failed run locally, and exits non-zero.
- **Client-side encryption:** The staged snapshot is compressed and encrypted
  client-side using `age` to the configured public recipient
  (`project/instrukt/spec/feature/configuration/layered-configuration`), so
  unattended runs hold no private decryption key on disk
  (`sdlc/policy/security-awareness`). Credentials and secrets are never written
  to logs or terminal output.
- **Multi-GB streaming and headroom:** Archiving, compression, encryption,
  upload, and server-side promotion stream without whole-file memory buffering
  so multi-GB homes (including multi-GB `state/history.db`) succeed above S3's
  5 GiB single-object copy limit. Before staging, backup checks available disk
  space against the shared-machine headroom requirement
  (`general/policy/shared-machine-resources`) and refuses without writing
  staging data if headroom would be violated; temporary staging files are
  cleaned up on both success and failure.
- **Outside-service availability:** Calls to the S3-compatible endpoint pass
  through the outside gate (`project/instrukt/design/architecture/outside-gate`,
  `project/instrukt/policy/checks-and-guards`) with one retry layer and bounded
  wait semantics; an outage leaves existing remote generations intact, cleans
  staging on a best-effort basis, and records the failed run locally.

### Offsite publication, verification and retention

- **Per-machine namespace:** Generations live under
  `<prefix>/<machine-id>/instrukt-home.<timestamp>.tar.<ext>.age` using this
  computer's `state/machine-id`, so multiple machines share one bucket without
  colliding.
- **Staged verify-promote-validate protocol:**
  1. Upload the encrypted archive to `_staging/<key>` using SigV4 with plain
     fixed-length or multipart upload framing compatible with DigitalOcean
     Spaces (never `aws-chunked` streaming-trailer checksum framing that
     non-AWS S3 providers mis-store).
  2. Read object metadata (`HEAD`) on `_staging/<key>` and verify an exact byte
     size match against the local encrypted archive. On mismatch or upload
     error, perform best-effort deletion of `_staging/<key>` and refuse to
     publish.
  3. Promote `_staging/<key>` to `<key>` via in-region server-side copy (single
     `CopyObject` up to 5 GiB; multipart `UploadPartCopy` above 5 GiB).
  4. Write the `_validated/<key>` marker object, then delete `_staging/<key>`.
- **Per-machine retention pruning:** After writing `_validated/<key>`, prune
  only this machine's namespace to the configured generation retention count
  (whose default is declared in
  `project/instrukt/spec/feature/configuration/layered-configuration`).
  Unvalidated generations are evicted before validated generations, oldest
  first within each group; pruning a validated generation removes its paired
  `_validated/<key>` marker as well. Generations belonging to other machine IDs
  are never touched.

### Guarded restore

- **Target selection:** Without `--from`, restore selects the newest
  `_validated/` generation for this computer's `machine-id` (or `--machine <id>`
  when recovering another machine's backup). `--from <key>` selects an explicit
  object key and permits recovering a named legacy or unvalidated generation.
- **Decryption, safe extraction and full integrity check:** Restore downloads
  the encrypted archive into temporary staging, decrypts it using the private
  identity supplied via `--identity <age-key>`, extracts entries while
  rejecting path traversal, absolute paths, and symlink escapes, and runs
  `PRAGMA integrity_check` on every restored `*.db` and `*.sqlite` database
  before modifying the destination. Any decryption, extraction, or integrity
  failure aborts without touching the destination.
- **Live-home lock guard:** When restoring into the active instrukt home
  (without `--into`), restore refuses while active instrukt state locks
  (`runner.lock`, `writer.lock`, or other held home state locks) are held,
  naming the holder where known and naming the door: stop instrukt services
  first, or pass `--into <dir>` to extract beside the live home for inspection
  or selective recovery (`project/instrukt/policy/checks-and-guards`).
- **Alternate target (`--into <dir>`):** Extracts the verified backup contents
  into `<dir>` without acquiring or modifying the live instrukt home.
- **Overwrite confirmation:** Restore prompts unconditionally for confirmation
  before modifying the destination unless `--yes` is supplied.

### Local bookkeeping, scheduling and alerting

- **Dedicated local bookkeeping:** Each backup run atomically records its
  outcome (attempt timestamp, success or failure status, published generation
  key when successful, and failure summary when failed) in a dedicated local
  state file outside `instrukt.db` and `writer.lock`
  (`sdlc/policy/state-storage`). `instrukt status` reads this local record
  without contacting remote storage
  (`project/instrukt/spec/feature/publication/instrukt-status`).
- **Failure and staleness detection:** While home backup is configured, a
  failed backup run or an elapsed interval since the last successful backup
  beyond the staleness threshold is an active backup anomaly condition,
  evaluated strictly from local bookkeeping without network calls and cleared
  once a subsequent backup succeeds. Raising that anomaly through
  `runner.notify` and listing it with its door in `instrukt events status` is
  owned by `project/instrukt/spec/feature/runner/anomaly-notification`.
- **Daily scheduling:** Convergence, inspection, and withdrawal of the owned
  daily OS timer that invokes `instrukt backup run` without starting an agent
  session belong to `project/instrukt/spec/feature/runner/module-installation`.

## Use cases

### UC-HB1: Back up an instrukt home consistently and encrypted offsite

```gherkin
Given an operator has configured encrypted offsite home backup on their computer
And the computer's instrukt home holds multiple active SQLite stores, machine settings and secrets, global research and exploration notes, a company home with local state beside its git-tracked checkout, and excluded caches, locks and stray recovery files
When the operator runs a home backup
Then every SQLite store in machine state and company homes is snapshotted consistently and verified before archiving
And the archive includes the irreplaceable machine state, research notes and company home local state while omitting caches, locks, live database sidecars, recovery copies, published docs and git-tracked company repository trees
And the encrypted archive is verified in remote staging, promoted to this computer's namespace, marked validated and cleaned from staging
And the archive cannot be read without the matching private decryption key
And local backup bookkeeping records the completed generation
But when a database fails consistency verification or the staged remote upload fails verification
Then nothing is published to the computer's namespace, existing validated generations remain intact, staged remote data is cleaned up and the failure is recorded locally
```

### UC-HB2: List and prune backup generations per machine

```gherkin
Given an operator's S3 bucket holds backup generations for two different computers under one prefix
And this computer's namespace holds both validated and unvalidated generations exceeding the configured retention count
When the operator lists backups for this computer
Then the listing shows this computer's generations newest first, distinguishing validated from unvalidated, and omits the other computer's generations
When the operator lists backups for the other computer
Then the listing shows the other computer's generations
When a new backup publishes for this computer
Then pruning retains only the configured number of generations for this computer, removing unvalidated generations before validated ones and removing paired validation markers
And the other computer's generations are untouched
```

### UC-HB3: Restore a home backup with integrity and live-home guards

```gherkin
Given an operator has a validated encrypted home backup in remote storage and holds its matching private decryption key
When the operator restores into a stopped home or an alternate target directory and confirms overwrite
Then the archive is decrypted, safely extracted and every restored SQLite database passes full integrity verification before the destination is modified
And restoring into an alternate directory leaves the live home untouched
But when the operator attempts to restore into a live home while instrukt services hold active locks
Then restore refuses before modifying the destination and names the options to stop services or restore into an alternate directory
And when the archive fails decryption, contains unsafe paths or holds a database that fails integrity verification
Then restore aborts without modifying the destination
```

### UC-HB4: Record backup failure and staleness from local bookkeeping

```gherkin
Given an operator has configured home backup on their computer
When a backup run fails
Then local backup bookkeeping records the failed attempt while preserving the last successful generation if one existed
And home backup reports an active failure condition from local state without contacting remote storage
When no backup has succeeded within the configured staleness threshold
Then home backup reports an active staleness condition from local state without contacting remote storage
When a subsequent backup succeeds
Then local backup bookkeeping updates to the new successful generation and clears the active backup anomaly condition
```

## Known caveats

- Decryption requires the operator's private `age` identity kept outside the
  backed-up home; unattended nightly runs never store the private key on disk,
  so losing the private identity makes remote archives unrecoverable.
- Git-tracked company repositories and project checkouts are intentionally
  excluded from home backup and are recovered from their git remotes; any
  uncommitted local work in those checkouts is not included.
- Restoring a home backup onto a fresh machine recovers irreplaceable state and
  configuration; regenerable publication outputs, editor schemas, blueprint
  graphs, and owned OS units are rebuilt afterward with `instrukt sync` and
  `instrukt install`.

## See Also

- [project/instrukt/spec/domain/runner](../../domain/runner.md)
- [project/instrukt/spec/feature/configuration/layered-configuration](../configuration/layered-configuration.md)
- [project/instrukt/spec/feature/configuration/owned-files](../configuration/owned-files.md)
- [project/instrukt/spec/feature/configuration/company-homes](../configuration/company-homes.md)
- [project/instrukt/spec/feature/runner/module-installation](module-installation.md)
- [project/instrukt/spec/feature/runner/anomaly-notification](anomaly-notification.md)
- [project/instrukt/spec/feature/runner/company-containers](company-containers.md)
- [project/instrukt/spec/feature/publication/instrukt-status](../publication/instrukt-status.md)
- [project/instrukt/design/architecture/outside-gate](../../../design/architecture/outside-gate.md)
- [project/instrukt/policy/checks-and-guards](../../../policy/checks-and-guards.md)
