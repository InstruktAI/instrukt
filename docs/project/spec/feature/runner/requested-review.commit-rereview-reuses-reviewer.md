---
description: An authoring session asks for the independent review of an exact committed worktree head by naming its coordinates only; instrukt starts or wakes the reviewer on an assignment it rendered from them, sends a revised head to the reviewer that recorded the earlier verdict, and returns the verdict to the session that asked.
---
# Requested Review — Spec

## What it is

Committed work that instrukt asks no review for on its own initiative — a
one-off worktree head, a human-directed branch — is reviewed when its authoring
session asks (`general/policy/commits`). A requested review is that ask, made
with `instrukt review start`. The requester names where the candidate is;
instrukt checks the candidate, renders the reviewer's assignment and starts or
wakes the reviewer on it itself. No words of the requester's reach the
reviewer, so the reviewer's mandate is its procedure
(`sdlc/procedure/lifecycle/work/review-commits`) and the candidate, as it is
when instrukt asks for a pull request's review
(`project/instrukt/spec/feature/runner/pull-request-review`).

The loop closes both ways. A revised head goes to the reviewer that recorded
the earlier head's verdict, which reviews its own findings' closure without a
cold start, and the verdict comes back to the session that asked without that
session asking for it.

A requested review is not a review request. A review request is the runner's
fact `runner.review_needed`, emitted on instrukt's own initiative and answered
by a standing reviewer; a requested review emits no event, and the runner
holds no review request for it.

## Canonical fields

### The command

```sh
instrukt review start --worktree <path> --branch <name> --head <sha> --base <sha> --report <path> \
  [--prior-report <path>] [--disposition <path>] [--source <pointer>]...
```

The ask concerns the project the caller stands in. It records no review
evidence and changes nothing in the repository.

### Coordinates

A requested review is asked for with coordinates. Each names a place or a
commit and carries no text of its own.

| Coordinate | Option | Names | Form | Present |
| --- | --- | --- | --- | --- |
| Worktree | `--worktree` | The checkout that holds the candidate. | An absolute path. | Always |
| Branch | `--branch` | The branch the candidate is on. | A branch name Git accepts. | Always |
| Head | `--head` | The exact candidate commit. | A full commit id. | Always |
| Base | `--base` | The pinned base, the commit the reviewed change starts from. | A full commit id. | Always |
| Report destination | `--report` | Where the reviewer writes its complete report. | An absolute path. | Always |
| Prior report | `--prior-report` | The report of the earlier review. | An absolute path. | On re-review |
| Disposition file | `--disposition` | The author's dispositions of the earlier findings, with their sources (`general/policy/commits`). | An absolute path. | On re-review of an unchanged head |
| Source pointer | `--source` | A source the reviewer reads beside the candidate: a snippet id, or a file at a commit of a repository. Any number of them. | A snippet id, or `repo@commit:path` with a full commit id and a relative path. | Optional |

No value holds a line break or another control character. The ask has no part
that takes narrative, a classification, a constraint, a summary or an
instruction. What an author wants a reviewer to know travels in the commit
bodies of the reviewed range or in the disposition file
(`general/policy/commits`), and the reviewer reads both as its procedure
directs. A source pointer adds to what the reviewer reads and removes nothing
from it (`sdlc/procedure/lifecycle/work/review-commits`).

The pinned base of a requested review is the pinned base work admits the
head's verdict against (`project/instrukt/spec/feature/work/github-apps`). For a head that shares history with the
project's canonical branch and has had no review request, that is its merge
base with the canonical branch, as the review procedure pins a one-off
candidate (`sdlc/procedure/lifecycle/work/review-commits`). The runner reads it
from work and derives none of its own.

### Kinds of ask

| Ask | Names | The head's verdict on this computer |
| --- | --- | --- |
| Review | No prior report, no disposition file | None stands here, and none was handed off. |
| Re-review of a revised head | The prior report, no disposition file | None stands here, and none was handed off. |
| Dispute of findings on an unchanged head | The prior report and the disposition file | It stands here, and its current version was recorded by a requested reviewer (Reaching the reviewer). |

The head's verdict here is the head verdict work records for this computer: one
that stands here with the session that recorded its current version, one an
export handed off, or none
(`project/instrukt/spec/feature/work/github-apps`). A head whose verdict was handed off admits no ask of
any kind, because work admits no further version of it here. A derived
approval and a verdict recognised as already published are no head verdict
and decide no kind of ask.

A disposition file without a prior report is refused under Form.

### Admission

A requested review is admitted only when all of these hold. They are facts of
the candidate and of what this computer holds for it, read when the review is
asked for.

| Check | Rule | The refusal names |
| --- | --- | --- |
| Form | Every value has the form of its coordinate and the ask is one of the kinds above. | The value and the form its coordinate takes. |
| Placement | The worktree is the project's registered checkout or a linked worktree of it, the named branch is checked out in it, and that branch's head is the named head. The report destination lies outside the worktree, in a directory that exists; a prior report and a disposition file are files that exist. | What the repository or the file system shows instead. |
| Cleanliness | The worktree holds no uncommitted change. | That the worktree is not clean. The door is committing or removing the change. |
| Range | The head is a commit of the repository that shares history with the project's canonical branch, and the named base is the pinned base. | The pinned base, or that the head shares no history with the canonical branch. |
| Review state | The head's verdict on this computer is as its kind of ask requires (Kinds of ask), read from work (`project/instrukt/spec/feature/work/github-apps`). | The verdict that stands, the session that recorded its current version and `instrukt review show`; for a verdict handed off, the export and its door (`project/instrukt/spec/feature/work/github-apps`). |
| Request | The commit has no live review request: none is queued, selected or handed over (`project/instrukt/spec/feature/runner/pull-request-review`). | The request with its standing, and the session that holds it once it was handed over. The door is that request's review, seen with `instrukt review show`. |
| Outstanding | No ask of the head is outstanding, and the requested reviewer the ask reaches has no turn in progress (Reaching the reviewer). The same ask repeated while it is outstanding is not refused: it returns that ask. | The outstanding ask or the turn in progress, and its reviewer session. |
| Runtime | The selected runtime starts a session under a fixed key (attaching an existing linked worktree when the candidate lives in one), sends it input under a fixed command identity whose acceptance it reports afterwards, and reports whether the session has a turn in progress (`project/instrukt/spec/feature/runner/session-continuity`). | The runtime and the capability it lacks. |
| Delivery | When the caller resolves to a session, a runner wake-up serves this computer: the shared minute wake-up is registered, or the operator schedules it, as `instrukt ticks status` reports (`launchd`, `systemd`, `cron` or `operator`), or the owned runner service runs `instrukt run` (`project/instrukt/spec/feature/runner/module-installation`). A caller that resolves to no session is owed no notice and needs none. | What `instrukt ticks status` reports instead, and its door: `instrukt install` for a missing or broken registration; for no demand, enabling the todos or events module of a project on this computer and then `instrukt install` (`project/instrukt/spec/feature/runner/module-installation`). |
| Procedure | `sdlc/procedure/lifecycle/work/review-commits` is active in the project's published set and declares the `commit-reviewer` role. | The procedure and what the published set shows instead. |

A refused ask starts no reviewer, sends nothing and records nothing.

### The reviewer's assignment

instrukt renders the assignment from the admitted coordinates in one fixed
wording, as a runner-authored `[Instrukt Assignment]` control frame. It holds,
in this order:

1. the procedure the reviewer executes, `sdlc/procedure/lifecycle/work/review-commits`,
   read with its closure, for the one-off worktree review it describes;
2. the role, `commit-reviewer`, and the project;
3. the coordinates, each under its name, in the order of the table above, with
   source pointers in the order they were given.

The same coordinates always render the same assignment, and it holds nothing
the requester composed. The reviewer of a requested review receives this
rendering as its assignment, and the ask offers no way to add to it. The
reviewer returns its outcome as its procedure directs for a one-off review
(`sdlc/procedure/lifecycle/work/review-commits`).

A requested review is answered by the procedure that answers a review instrukt
asks for on its own initiative, under the role that procedure declares
(`sdlc/procedure/lifecycle/work/review-commits`), and covers the commits after
the pinned base up to the exact head. The reviewer is independent of the
candidate's authors as its procedure and the admission of its verdict require
(`project/instrukt/spec/feature/work/github-apps`): it is a session instrukt
started for a requested review, and its role commits nothing.

### Reaching the reviewer

A requested reviewer is an instrukt-owned session started under the start key
`instrukt:review:<encoded-project>:<head>` of the head it was started for
(`project/instrukt/spec/feature/runner/session-continuity`), in the candidate's
worktree — attaching it as an existing worktree of the project's registered
checkout when it is a linked worktree, or in the registered checkout when the
candidate lives there — under the role `commit-reviewer`. It keeps that key
when a later ask reaches it for another head. The ask holds the key of the
reviewer it reaches from admission to the runtime's acceptance.

A commit's reviewer of record is the session that recorded the current version
of its verdict that stands here, as work answers it
(`project/instrukt/spec/feature/work/github-apps`). An ask reaches an earlier
head's reviewer of record through the reviewer wake addressed by that reviewed
head (`project/instrukt/spec/feature/runner/session-continuity`). Which
reviewer an ask reaches follows from its kind:

| Ask | The requested reviewer it reaches |
| --- | --- |
| Review | The head's own: the requested reviewer under the head's start key. |
| Re-review of a revised head | The earlier head's reviewer of record, when it is a requested reviewer. The earlier head is the commit whose verdict standing here has a version whose report has the prior report's SHA-256 digest, as work answers it. When no version has it, the earlier head is the head of the project's latest ask whose report destination is the prior report, the two paths compared in canonical form. |
| Dispute of findings on an unchanged head | The head's reviewer of record, which Review state requires to be a requested reviewer. |

A re-review has no reviewer of record to reach when neither the prior report's
digest nor its path identifies an earlier head, when the earlier head's verdict
does not stand here, or when its reviewer of record is not a requested
reviewer: reuse is not possible. It then reaches the head's own requested
reviewer, as a review does, and the reply names which of these held.

| The requested reviewer it reaches | What an admitted ask does |
| --- | --- |
| None yet | Starts the head's own requested reviewer with the rendering as its opening input. |
| Exists and has no turn in progress | Wakes it: sends it the rendering under the ask's command identity, and the rendering opens its next turn. |
| Exists and has a turn in progress | Nothing: the ask is refused under Outstanding. |
| Its session was archived or deleted | For a review, starts a successor under the same key, as session-continuity succeeds a generation. For a re-review, starts the head's own requested reviewer, or its successor when that one was archived or deleted too. A dispute is refused under Review state: no session that recorded the verdict remains to judge it. |

An ask is outstanding from its admission until the turn its rendering opened
ends. The same ask — the same coordinates — repeated while it is outstanding
returns that ask and starts or wakes nothing new. Each head's verdict stays
that head's own, whichever session records it.

A start is the keyed start of session-continuity, whose identities are fixed
by the start key and generation before anything reaches the runtime. A wake is
session-continuity's keyed wake of the session the reviewer's start key binds:
it carries a command identity derived from that key, the generation it is sent
to and the ask, recorded with the ask before anything reaches the runtime
(`project/instrukt/spec/feature/runner/session-continuity`). The
runtime keeps a repeated command identity from opening a second turn.

### Beside review requests

A requested review holds no review request; work admits its verdict under the
same head-verdict rules as any other review
(`project/instrukt/spec/feature/work/github-apps`). The ask is refused while the
commit has a live review request
(`project/instrukt/spec/feature/runner/pull-request-review`). The runner's own
decisions do not read requested reviews: a commit that becomes owed while its
requested reviewer works is asked for as any other owed commit
(`project/instrukt/spec/feature/runner/pull-request-review`), and the first
verdict recorded settles the commit.

### What the requester receives

An admitted ask returns within its own wait, once the runtime accepted the
start or the wake. The reply names the head, the pinned base, the kind of ask,
the requested reviewer's session and start key, whether this ask started it or
woke it, for a re-review the earlier head, whether the prior report's digest
or its path identified it, and whether the ask reached its reviewer of record
or why it did not, the report destination, and where the
outcome arrives: in the report destination, in `instrukt review show <head>`
once the reviewer records its verdict, and at the session that asked
(Returning the outcome). It names that session, or states that the caller
resolves to none, and states when `instrukt ticks status` finds the runner's
wake-up stale. The reply does not wait for the review.

When the runtime does not accept the start or the wake within the command's
wait, the ask is refused, naming the runtime and its door: the same command.
It replays the ask with the identities the first one recorded
(`project/instrukt/spec/feature/runner/session-continuity`). A wake is sent again under its
command identity, which the runtime accepts once: a wake it already accepted
opens no second turn and is reported accepted
(`project/instrukt/spec/feature/runner/t3-contract`). Nothing recognises a wake
by its text, because the same coordinates always render the same text. Only
that command replays an ask's start or wake; nothing in the background
delivers it.

### Returning the outcome

The ask records the session that asked, when the caller resolves to one
(`project/instrukt/spec/feature/runner/session-continuity`). A caller that
resolves to none is answered by the report destination and
`instrukt review show <head>` alone, and nothing refuses it for that
(`general/concept/lineage-vs-authorship`).

Once work has recorded a version of a head's verdict, it hands the version to
the runner (`project/instrukt/spec/feature/work/github-apps`). The runner then
queues a verdict notice for each session recorded as asking for that head, into
its input delivery to that session
(`project/instrukt/spec/feature/runner/procedure-event-delivery`), the route
approval attention uses. That queuing is one read of the head's asks and one
state-database write. The route delivers and retries each notice on the
runner's wake-ups, and the recording waits for no delivery. A notice carries
the head, the pinned base, the verdict, the version's number and report digest,
the reviewing session, the report destination of that session's ask and
`instrukt review show <head>`. It carries no report text and nothing of a
conversation. Its identity is made of the project, the head, the version's
number and report digest, and the session, so a session is sent each version
once. Only a recorded version is sent, so a reviewer that records nothing sends
nothing. A judged dispute reaches its asker as the version its reviewer
records (`sdlc/procedure/lifecycle/work/review-commits`).

A notice whose session is archived or deleted before it is delivered is closed
as undeliverable with that cause and is never sent again. `instrukt events
status` lists it.

### Verdict

Asking for a review records no verdict and no review evidence. The reviewer
records its verdict with `instrukt review record`, admitted under the rules
that bind every head verdict (`project/instrukt/spec/feature/work/github-apps`),
and the runner returns it to the session that asked (Returning the outcome).

### Re-review

A revised candidate is a new head, and its verdict is that head's own. Its
review is asked for naming the prior report, and reaches the reviewer of
record of the earlier head (Reaching the reviewer), which reviews the new head
under its procedure, from the closure of its own findings and the change since
the earlier head. A dispute of findings without a new commit names the
unchanged head, the prior report and the disposition file, and reaches that
head's reviewer of record, the session that recorded its verdict. That session
judges it under its procedure, which records every judgment as a new version
(`sdlc/procedure/lifecycle/work/review-commits`), and may revise the verdict
it recorded while that verdict stands locally
(`project/instrukt/spec/feature/work/github-apps`). Both carry
coordinates only.

### Requested review records

The runner keeps each admitted ask in a store of its own.

| Store | Holds | Written by | Write-isolation unit |
| --- | --- | --- | --- |
| `state/requested-review.db` | The admitted asks. | `instrukt review start` only: a command its caller waits on. | `state/requested-review.lock` |

| Property | Value |
| --- | --- |
| Holds | Per project and head: each admitted ask with its kind, its coordinates, its rendering and that rendering's SHA-256 digest, the session that asked or that none resolved, for a re-review the earlier head it was traced to, the start key of the requested reviewer it reaches, which is another head's when it reaches an earlier head's reviewer of record, whether a start or a wake delivers it, a wake's command identity, the generation it reached, and when it was admitted and accepted. |
| Nature | Authoritative: the requested reviewer's assignment, from which its role, project and placement resolve from the latest ask under its start key (`project/instrukt/spec/feature/configuration/roles-and-capabilities`), and the only record of which session asked. |
| Owner | The runner; written only by `instrukt review start`. |
| Loss cost | The requested reviewer resolves no assignment and records no verdict, and its verdict no longer reaches the session that asked: lost intent. |
| Medium | A SQLite database of its own with its own migrations: the asks grow with every head asked for, are read by head, by start key and by report destination, and a head's asks change together. |
| Write isolation | Its own lock, taken only by `instrukt review start` and held for each of that command's transactions, never while the command waits on the runtime or takes the state database's writer lock. No background writer takes it, and no write to this store takes the writer lock (`sdlc/policy/state-storage`). Readers, the requested reviewer's assignment resolution and `instrukt review record` among them, take no lock and never migrate it; only `instrukt review start` brings it to its current version. |
| Recording | The ask, with the session that asked and a wake's command identity, is recorded before its start or wake reaches the runtime, and its acceptance and generation once the runtime accepted it. The start key joins it to the runner's runtime binding, which the keyed start records in the state database as every keyed start does. |
| Notices | Queued by the runner, once work hands it a recorded version (`project/instrukt/spec/feature/work/github-apps`), into its input deliveries to the sessions that asked, in the state database (`project/instrukt/spec/feature/work/state-database`). This store records nothing of them. |
| Scope | One computer: the requested reviewer runs on this computer's runtime. Moving review evidence between computers moves no ask (`project/instrukt/spec/feature/work/github-apps`). |
| Lifecycle | A head's asks are current while the canonical branch does not contain the head, and history once it does, as the head's review evidence is. History is retired after the retention period of review evidence history (`project/instrukt/spec/feature/work/github-apps`), inside `instrukt review start`. |

## Use cases

### UC-RQ1: A developer's committed work is reviewed from its coordinates alone

```gherkin
Given a developer's agent has committed work on a branch in a clean worktree of the developer's project
When the agent asks for an independent review of that exact head
Then an independent reviewer is started on an assignment that names where the work is and holds nothing the agent wrote
But when the worktree holds uncommitted work or the branch is no longer at the named head
Then the review is not started and the agent is told what differs and how to repair it
```

### UC-RQ2: A developer's agent disputes findings on an unchanged head

```gherkin
Given a developer's agent received a needs-work verdict from the reviewer it asked for
When the agent asks again for the same head, naming the prior report and its dispositions
Then the reviewer that recorded the verdict judges the dispositions on an assignment that names those files and holds nothing the agent wrote
And that reviewer's judgment is recorded as a new version of the verdict, whether or not the verdict changes, and reaches the agent
But when the agent pushes a fixing commit instead and asks for its review naming the prior report
Then the reviewer that recorded the earlier verdict reviews the fixing head, on an assignment that names the new head and the prior report and holds nothing the agent wrote
And the fixing head's verdict is recorded as that head's own
But when that reviewer's session was archived or deleted
Then a fresh reviewer reviews the fixing head
```

### UC-RQ3: A developer's pull-request head under automatic review is not reviewed twice

```gherkin
Given a developer's project has opted in to automatic review and a pull request's head has a review request in flight
When the developer's agent asks for an independent review of that head
Then no second reviewer is started and the agent is told which review is in flight and where its verdict will show
```

### UC-RQ4: A developer's agent learns its review's outcome without asking for it

```gherkin
Given a developer's agent asked for the independent review of a head
When the reviewer records its verdict
Then the agent receives the verdict, with where its report is, without checking for it
And when the reviewer later records a revised verdict for that head, the agent receives that one too
```

## Known caveats

- Names inside coordinates — a branch name, a file name, a pointer's path — are
  the requester's and reach the reviewer, whose procedure reads them as
  untrusted data (`sdlc/procedure/lifecycle/work/review-commits`).
- A session can still start a reviewer by other means. Such a review is no
  requested review: nothing checks its assignment, and its verdict is admitted
  under the same rules (`project/instrukt/spec/feature/work/github-apps`).
- The requested reviewer is a native session. Input another session sends it
  through the runtime is not part of its assignment, and its procedure reads it
  as untrusted data.
- A commit that becomes owed under automatic review while its requested
  reviewer works is reviewed twice; the verdict recorded first settles it
  (`project/instrukt/spec/feature/work/github-apps`).
- A dispute reaches only the requested reviewer that recorded the verdict.
  Once that session is archived or deleted, or when the verdict was recorded
  by a reviewer started by other means, a dispute is refused and a revised
  head is the way forward.
- A re-review reaches the earlier reviewer only while its thread is kept.
  Once thread retention has archived it
  (`project/instrukt/spec/feature/runner/thread-retention`), a fresh reviewer
  reviews the revised head from a cold start.
- A requested reviewer that ends without recording a verdict sends nothing. The
  same ask again reaches the reviewer anew.
- A notice is queued after the verdict is recorded, not with it. If the command
  stops between the two writes, that version's notice is never sent, and the
  verdict stays readable with `instrukt review show <head>`.
- A notice reaches the session that asked as an instrukt delivery, which opens
  that session's next turn when it is idle and waits behind its turn in
  progress otherwise
  (`project/instrukt/spec/feature/runner/procedure-event-delivery`).

## See Also

- [project/instrukt/spec/feature/runner/pull-request-review](pull-request-review.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/runner/procedure-event-delivery](procedure-event-delivery.md)
- [project/instrukt/spec/feature/runner/thread-retention](thread-retention.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [sdlc/procedure/lifecycle/work/review-commits](../../../../domain/sdlc/procedure/lifecycle/work/review-commits.md)
- [general/policy/commits](../../../../general/policy/commits.md)
