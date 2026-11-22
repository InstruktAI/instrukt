---
description: Role-selected GitHub App bot identities, protected keys and installation tokens for visible bot authorship, independent native reviews and trusted record recovery.
---
# GitHub Apps — Spec

## What it is

instrukt writes to GitHub as configured GitHub Apps. The bot is visible on
records, issues, bugs, board updates, pull requests and reviews. Authoring and
review use separate bots, so GitHub's native prohibition on author self-approval
does not prevent independent review. Account lineage is distinct from who did
the work (`general/concept/lineage-vs-authorship`).

## Canonical fields

### App inventory and role selection

The machine-global `instrukt.yaml` declares `github.apps`, a map of public App
profiles, and `github.project_app`, the project-identity profile reference.
Each profile records `app_id`, `slug`, `bot: {id, node_id, login}` (the stable
bot account identity), and `private_key` (a protected secret reference). The key
reference is `${NAME}` into the protected machine-local secrets file, never
the key itself, a repository-selected path or an environment fallback.
Configuration validates shapes, distinct App/bot identities and all references;
the GitHub boundary verifies the configured actor against GitHub.

The actual recorded session role selects `roles.<role>.github_app` in
`~/.instrukt/agents.yaml`. An omitted mapping uses `github.project_app`,
including an unknown role resolved through `default`. No per-task model override,
command argument or profile order selects the GitHub actor. Background record,
issue, bug and board maintenance uses the project identity. Role-driven pull
request authoring and reviews use the selected role identity. Human work
performed directly on GitHub keeps its human identity; instrukt's shared-record
maintenance still uses the project App. This contract changes no human login/
account association or runtime permission mode.

| Profile reference | Shipped identity | Responsibility |
| --- | --- | --- |
| `project` | `instruktai[bot]` | Shared records, issues, bugs and the board; roles without a specific mapping. |
| `author` | `instruktai-author[bot]` | Authoring roles, their pull requests and bot-authored commits. |
| `reviewer` | `instruktai-reviewer[bot]` | Reviewer roles and their native GitHub reviews. |

The shipped role compositions live in
`project/instrukt/spec/feature/configuration/roles-and-capabilities`. An
independent reviewer uses a different App/bot from the pull request author;
using a different session of the same App does not satisfy GitHub's native
restriction. Review stamps retain their own independence and exact-evidence
requirements; different bots alone do not prove review occurred.

### Configuration skill and installation

The configuration skill reuses the three existing TeleClaude Apps on the
installation that already has them, verifying their configured identity and
repository installation instead of creating duplicates. On other installations
it guides creation and installation of the three Apps and records their actual
public identities; globally unique slugs can differ from the shipped names.
It writes private keys only through `instrukt secrets set`, into
`~/.instrukt/secrets.env` under the owned-files contract. No key or token is
carried in a configuration report, repo file or assignment brief.

Setup verifies the App installation reaches the project's repository and has
the permissions required by the operation: records/issues/artifacts and board
maintenance, code/pull-request authoring, or reviews/check observation as
applicable. Missing installation, permissions or an API capability remains an
explicit incomplete setup with its repair door. No personal token or `gh`
login is silently substituted for the selected App.

### Installation credentials

The shared GitHub boundary resolves the chosen App's key from the protected
file on interactive and scheduled paths alike. It authenticates as that App
with a signed JWT, discovers/verifies its installation for the target
repository, and requests an installation access token with the required
repository and permission scope. It verifies returned identity, repository
access, permissions and usable expiry before use. Tokens expire and are
renewed from the same App identity; renewal is not a switch of actor.
Account/project board operations use the installation's applicable account
permissions rather than assuming a repository token grants board access.

Private keys remain protected-file material. Issued tokens are short-lived
in-memory credentials, never persisted in records, runtime assignment evidence,
git remotes, logs or configuration. An instrukt-owned child needing GitHub
credentials receives only the bounded installation credential for that
operation, never the App key. Raw agent `gh`/git use remains with the runtime
permission and credential integration; this does not add a general secret
export command or a permission gate for those external tools.

Absent keys, an unavailable installation, identity/scope mismatch or failed
issuance leaves the GitHub operation visibly pending/refused with a secret-safe
cause. Rate limits retain their retry boundary. Locally accepted work and its
pending reflection survive; no credential failure is reported as remote success
or as proof that an ambiguous write had no effect. The record's existing
revision and retry-convergence rules remain in force.

### Read-only reflection readiness

Work verifies the project App, protected key, repository installation,
record and account-board permission scopes, expiry and stable bot identities.
It reads repository issues, existing issue artifacts when an issue is available,
and the configured board or its account's project listing. Newer pages containing
only pull requests do not exclude older issues from artifact readiness checks.
Unavailable access or reflection capabilities are unmet findings with the
configuration repair door.
Read-only readiness creates no issues, artifacts, boards, fields or views.
It establishes declared write permissions and available read surfaces; it cannot
prove a future remote write, or artifact access on an empty repository, without
performing a mutation.

### Restore trust and commit authorship

Record restoration accepts an author matching a configured App's verified
stable bot identity even when GitHub reports association `NONE`. A display
login or bot-shaped name alone establishes no trust. Repository owner/member/
collaborator records retain their existing admission. Trust admits record
envelope processing; it bypasses neither schema-version checks, digests,
revision history nor approval attribution. Bot trust grants no human approval
or hold-control authority.

The App boundary supplies verified bot actor facts; commit-author resolution,
precedence and refusal belong to
`project/instrukt/spec/feature/repository/attributed-commit`.

<!-- planned:event-driven-pr-review-gate -->
### Review verdict on the reviewed commit

A verdict of an automatically started pull-request review
(`project/instrukt/spec/feature/runner/pull-request-review`) is published by the
reviewer bot in two places:

| Verdict | Pull-request review | Review status on the reviewed commit |
| --- | --- | --- |
| Approve | Approving review | Success |
| Needs work | Change-requesting review carrying the findings | Failure |

The status is named `instrukt/review`. It names the exact commit that was
reviewed; a later head never inherits it. The reviewer bot is the status's only
publisher; a status of the same name from another identity is not a verdict.
The reviewer identity is the App the `commit-reviewer` role selects.

The verdict is a judgment of the commit; the pull-request review is its
delivery to one pull request. Every eligible pull request whose current head is
that commit receives its own review of the one verdict, and the commit receives
one status.

### Review evidence

Work keeps three kinds of review evidence for exact commits that no todo's
record carries.

| Evidence | Records | Recorded by |
| --- | --- | --- |
| Head verdict | An independent review's outcome for one commit: the commit, the base it was reviewed against, the verdict, the complete report and the reviewing session. | The reviewing session, with `instrukt review record`. |
| Exemption | The decision of a commit's own author that it owes no independent review under `general/policy/commits`, with its stated basis. | The session that authored the commit, with `instrukt review exempt`. |
| Derived approval | Workflow's finding that an integration candidate's content is covered (`project/instrukt/spec/feature/workflow/integration-gate`), with the canonical head it was derived against and every covering item. | The runner's tick, through work's command contract. No agent command records one. |

| Command | Contract |
| --- | --- |
| `instrukt review record --head <sha> --verdict approve\|needs_work --report <file\|-> [--base <sha>]` | Records a head verdict. A later verdict for the same commit is a new version: the latest is current and earlier ones are kept as history. |
| `instrukt review exempt --commit <sha> --basis <text>` | Records an exemption for a commit the canonical branch does not contain yet. |
| `instrukt review show <sha>` | The commit's verdict versions, exemption or derived approval, and each publication with its state and the cause of an outstanding or refused one. |

### Admission of review evidence

`instrukt review record` admits a verdict only when all of these hold. They are
facts of the reviewed object and bind every caller, a session without a role
included (`project/instrukt/spec/feature/configuration/roles-and-capabilities`).

| Invariant | Rule | The refusal names |
| --- | --- | --- |
| Repository | `--head` is the full id of a commit present in the repository of the project the caller stands in. | The commit and the project. |
| Review target | Either the runner delivered a review request for exactly this project and commit to the calling session (`project/instrukt/spec/feature/runner/pull-request-review`), or no review request was issued for this commit to any session and `--base` names the commit the reviewed change starts from. A commit whose request was delivered to another session is that session's to record. | The session that holds the request. |
| Independence | The calling session authored none of the commits under review: no commit after the base up to the head carries it in its `Session:` trailer (`project/instrukt/spec/feature/repository/attributed-commit`). The base of a requested review is the head's merge base with the canonical branch. | The commits the caller authored; the door is a review by a session that authored none of them. |
| Report | A non-empty report is supplied. It is stored whole with the version. | The missing report. |

The caller names no pull request. Where a verdict is delivered follows from
which eligible pull requests have the commit as their current head, as the
runner observed them. A request delivered for an earlier head of a pull request
stays recordable after the pull request has moved on; its verdict is that
commit's and no other's.

An exemption is admitted only from the session the commit's `Session:` trailer
names. It counts in candidate coverage alone and never gives a pull request's
head a status. A derived approval is admitted only for the fetched integration
head while the canonical branch does not contain it, and only with the coverage
finding it rests on.

### Verdict publication

A verdict is recorded before anything of it is published, and recording does
not wait for GitHub. Each tick publishes what is outstanding for an opted-in
project, whether or not that project's todos module is on.

| Publication | Published once |
| --- | --- |
| The review status on the commit | GitHub has the commit in the project's repository. A verdict of a commit GitHub does not have yet waits, and that is no failure. |
| The pull-request review on each eligible pull request whose current head is the commit | The runner observed that pull request at that head. |
| A derived approval's review status on the candidate commit | The derived approval is recorded. It publishes no pull-request review. |

Only a commit's current version is published; a version replaced before it was
published is not. Before writing, publication recognises what GitHub already
carries from the reviewer bot: its status of that name on the commit, and its
review of that commit on that pull request. A write that already landed is not
repeated. A verdict found on GitHub with one of its two marks missing is
completed, never reviewed again: the status from the review, and the review
from the report on the computer that holds it.

An unavailable GitHub leaves a publication outstanding with its cause, and the
next tick retries it. A destination GitHub refuses for good, such as a pull
request closed before its review was written, is recorded as refused with its
cause and is not retried.

### Review evidence storage

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/review.db` | Head verdict versions with their reports, and exemptions. Authoritative. | `instrukt review record` and `instrukt review exempt` only: commands their caller waits on. | `state/review.lock` |
| `state/review-publication.db` | Derived approvals with their coverage, and per verdict version and destination whether GitHub carries it, with the cause of an outstanding or refused one. | The tick only. | `state/review-publication.lock` |

No background work writes the evidence store and no command a caller waits on
writes the publication store, so recording a verdict never waits behind a tick
(`sdlc/policy/state-storage`). Neither store takes the state database's writer
lock, and readers take no lock. The publication store refers to evidence by
commit and version, and a publication record that is missing is rebuilt from
what GitHub carries, so the two stores share no transaction.

Evidence is current for as long as it can still admit something: a commit's
current verdict, and an exemption, while the canonical branch does not contain
the commit and a branch of the project still does; an outstanding publication
until it is published or refused; a derived approval while its candidate is
the integration head. Everything else is history: replaced versions, evidence
of a commit the canonical branch contains or no branch reaches any more, and
completed publications. History is retired after a retention period. Current
evidence and outstanding publications are never retired by age. The evidence
store retires its history inside a recording command, the publication store
inside a tick.

Both stores are per computer. GitHub carries a published verdict to other
computers: the status carries the verdict and its publisher, and the
pull-request review carries the findings. A computer that takes over a project
recognises published verdicts and derives the rest again: an owed head it finds
no verdict for is asked for again, and a candidate's coverage is derived from
the statuses its heads carry.

### Readiness for automatic review

An opted-in project needs two scoped installation credentials. No other
credential changes.

| Credential | Identity | Permissions | Used for |
| --- | --- | --- | --- |
| Review observation | The project identity | Read access to pull requests and to commit statuses | The tick's pull-request listing and its reads of statuses and reviews. |
| Review publication | The reviewer identity | Write access to pull requests and to commit statuses | The pull-request review and the review status. |

Work verifies both by requesting each credential with exactly those
permissions, where the project's opt-in is admitted and in
`instrukt install --check`
(`project/instrukt/spec/feature/configuration/layered-configuration`,
`project/instrukt/spec/feature/runner/module-installation`). A missing
installation or permission is an unmet finding naming the App, the repository,
the missing permission and the configuration repair door. Verification writes
no state. The credentials that observe CI and that reflect build reviews are
unchanged: a project that has not opted in needs neither permission.
<!-- /planned:event-driven-pr-review-gate -->

## Use cases

### UC-GA1: See bot authorship and receive an independent native review

```gherkin
Given a developer has configured distinct project, author and reviewer Apps
When an author role opens its implementation pull request and a reviewer role approves it
Then GitHub shows the author bot and the different reviewer bot
And shared record, issue and board maintenance shows the project bot
```

### UC-GA2: Recover reflected work written by the configured bots

```gherkin
Given a developer's reflected work was written by a configured verified App bot
When they rebuild their local record from GitHub
Then the bot's NONE association does not exclude its valid record revisions
And schema, artifact and evidence checks still apply
But a similarly named unconfigured actor gains no record trust
```

<!-- planned:event-driven-pr-review-gate -->
### UC-GA3: See an automatic review's verdict where GitHub can enforce it

```gherkin
Given a developer's pull request head has been reviewed automatically
When the reviewer's verdict is published
Then the pull request shows the reviewer bot's approving or change-requesting review with its findings
And the reviewed commit carries the reviewer bot's matching success or failure review status
But a later head of that pull request carries no status from that verdict
```
<!-- /planned:event-driven-pr-review-gate -->

## Known caveats

- GitHub App installation/permission availability and provider artifact/board
  capabilities still bound remote work; token issuance supplies no missing API.
- Human approval attribution remains the approvals contract. App configuration
  does not grant a bot permission to speak for an authorized human.

<!-- planned:event-driven-pr-review-gate -->
- A head verdict's report reaches other computers only inside a pull-request
  review. A verdict of a commit without a pull request, an exemption and a
  derived approval's coverage stay on the computer that recorded them, where
  `instrukt review show` reads them; GitHub carries the status alone.
- A computer that takes over a project while a verdict there is still
  unpublished cannot see it and asks for that head's review again.
<!-- /planned:event-driven-pr-review-gate -->

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [project/instrukt/spec/feature/configuration/configuration-skill](../configuration/configuration-skill.md)
- [project/instrukt/spec/feature/configuration/owned-files](../configuration/owned-files.md)
- [project/instrukt/spec/feature/work/github-record](github-record.md)
- [project/instrukt/spec/feature/repository/attributed-commit](../repository/attributed-commit.md)
- [general/concept/lineage-vs-authorship](../../../../general/concept/lineage-vs-authorship.md)
- https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app
- https://github.com/actions/create-github-app-token — GitHub's reference token issuance and bot commit-author example.
