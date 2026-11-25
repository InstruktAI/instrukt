---
description: An authoring session asks for the independent review of an exact committed worktree head by naming its coordinates only, and instrukt starts the reviewer on an assignment it rendered from them.
delivered_by:
  - fixed-shape-commit-review-dispatch
---
# Requested Review — Spec

## What it is

Committed work that instrukt asks no review for on its own initiative — a
one-off worktree head, a human-directed branch — is reviewed when its authoring
session asks (`general/policy/commits`). A requested review is that ask, made
with `instrukt review start`. The requester names where the candidate is;
instrukt checks the candidate, renders the reviewer's assignment and starts the
reviewer on it itself. No words of the requester's reach the reviewer, so the
reviewer's mandate is its procedure
(`sdlc/procedure/lifecycle/work/review-commits`) and the candidate, as it is
when instrukt asks for a pull request's review
(`project/instrukt/spec/feature/runner/pull-request-review`).

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
head's verdict against (`project/instrukt/spec/feature/work/github-apps`,
Admission of review evidence, Range). For a head that shares history with the
project's canonical branch and has had no review request, that is its merge
base with the canonical branch, as the review procedure pins a one-off
candidate (`sdlc/procedure/lifecycle/work/review-commits`). The runner reads it
from work and derives none of its own.

### Kinds of ask

| Ask | Names | The head |
| --- | --- | --- |
| Review | No prior report, no disposition file | Carries no current verdict on this computer. |
| Re-review of a revised head | The prior report, no disposition file | Carries no current verdict on this computer. |
| Dispute of findings on an unchanged head | The prior report and the disposition file | Carries a current verdict on this computer, recorded by this head's requested reviewer (Starting the reviewer). |

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
| Review state | The head's current verdict on this computer is as its kind of ask requires (Kinds of ask), read from work (`project/instrukt/spec/feature/work/github-apps`, Review evidence). | The current verdict and `instrukt review show`; for a dispute, the session that recorded it. |
| Request | The commit has no live review request: none is queued, selected or handed over (`project/instrukt/spec/feature/runner/pull-request-review`, One review per commit). | The request with its standing, and the session that holds it once it was handed over. The door is that request's review, seen with `instrukt review show`. |
| Outstanding | No ask of the head is outstanding, and the head's requested reviewer has no turn in progress (Starting the reviewer). The same ask repeated while it is outstanding is not refused: it returns that ask. | The outstanding ask or the turn in progress, and its reviewer session. |
| Runtime | The selected runtime starts a session under a fixed key, sends it input, reports whether it has a turn in progress and recognises the caller (`project/instrukt/spec/feature/runner/session-continuity`, Runtime choice and feature availability). | The runtime and the capability it lacks. |
| Procedure | `sdlc/procedure/lifecycle/work/review-commits` is active in the project's published set and declares the `commit-reviewer` role. | The procedure and what the published set shows instead. |

A refused ask starts no reviewer, sends nothing and records nothing.

### The reviewer's assignment

instrukt renders the assignment from the admitted coordinates in one fixed
wording, as a runner-authored assignment input
(`project/instrukt/spec/feature/runner/conversation-links`, `[Instrukt
Assignment]`). It holds, in this order:

1. the procedure the reviewer executes, `sdlc/procedure/lifecycle/work/review-commits`,
   read with its closure, for the one-off worktree review it describes;
2. the role, `commit-reviewer`, and the project;
3. the coordinates, each under its name, in the order of the table above, with
   source pointers in the order they were given;
4. where the outcome is returned: the requesting session, as the selected
   runtime recognises the caller, or no session when the caller is not one.

The same coordinates from the same requesting session always render the same
assignment, and it holds nothing the requester composed. The reviewer of a
requested review receives this rendering as its assignment, and the ask offers
no way to add to it.

A requested review is answered by the procedure that answers a review instrukt
asks for on its own initiative, under the role that procedure declares
(`sdlc/procedure/lifecycle/work/review-commits`), and covers the commits after
the pinned base up to the exact head. The reviewer is independent of the
candidate's authors as its procedure and the admission of its verdict require
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence): it is a session instrukt started for this head, and its role
commits nothing.

### Starting the reviewer

Each head asked for has one requested reviewer: an instrukt-owned session under
the start key `instrukt:review:<encoded-project>:<head>`
(`project/instrukt/spec/feature/runner/session-continuity`, Start keys and
locks), started in the project's registered checkout under the role
`commit-reviewer`. The ask holds that key's lock from admission to the
runtime's acceptance.

| The head's requested reviewer | What an admitted ask does |
| --- | --- |
| None yet | Starts it with the rendering as its opening input. |
| Exists and has no turn in progress | Sends it the rendering, which opens its next turn. |
| Exists and has a turn in progress | Nothing: the ask is refused under Outstanding. |
| Its session was archived or deleted | Starts a successor under the same key, as session-continuity succeeds a generation, for a review or a re-review. A dispute is refused under Review state: the successor did not record the verdict. |

An ask is outstanding from its admission until the turn its rendering opened
ends. The same ask — the same coordinates from the same requesting session —
repeated while it is outstanding returns that ask and starts or sends nothing
new. A re-review of a revised head is a new head and has its own requested
reviewer.

### Beside review requests

A requested review holds no review request, so work admits its verdict as a
one-off review's (`project/instrukt/spec/feature/work/github-apps`, Admission
of review evidence, Request). The ask is refused while the commit has a live
review request, because work then admits only that request's reviewer. The
runner's own decisions do not read requested reviews: a commit that becomes
owed while its requested reviewer works is asked for as any other owed commit
(`project/instrukt/spec/feature/runner/pull-request-review`, Review requests),
and from then on only that request's reviewer records its verdict.

### What the requester receives

An admitted ask returns within its own wait, once the runtime accepted the
start or the send. The reply names the head, the pinned base, the kind of ask,
the requested reviewer's session and start key, whether this ask started it or
sent to it, the report destination, and where the outcome arrives: in the
report destination, in `instrukt review show <head>` once the reviewer records
its verdict, and as the reviewer's return to the requesting session when the
rendering names one. The reply does not wait for the review.

When the runtime does not accept the start or the send within the command's
wait, the ask is refused, naming the runtime and its door: the same command,
which replays the same start or send with the identities the first ask fixed
(`project/instrukt/policy/checks-and-guards`).

### Verdict

Asking for a review records no verdict and no review evidence. The reviewer
records its verdict with `instrukt review record`, admitted under the rules
that bind every head verdict (`project/instrukt/spec/feature/work/github-apps`,
Admission of review evidence), and returns its outcome as its procedure
directs.

### Re-review

A revised candidate is a new head. Its review is asked for as that head's, and
names the prior report. A dispute of findings without a new commit names the
unchanged head, the prior report and the disposition file, and is sent to that
head's requested reviewer, the session that recorded its verdict. That session
judges it under its procedure; the turn the dispute opened is no review
request's, so a revised verdict it records is admitted as a revision
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence, Settlement). Both carry coordinates only.

### Requested review records

The runner keeps each admitted ask in the state database
(`project/instrukt/spec/feature/work/state-database`, Runner tables).

| Property | Value |
| --- | --- |
| Holds | Per project and head: each admitted ask with its kind, its coordinates, the requesting session, its rendering and the start or send that delivered it. |
| Nature | Authoritative: the requested reviewer's assignment, from which its role, project and placement resolve (`project/instrukt/spec/feature/configuration/roles-and-capabilities`, Caller and object resolution), and the only record of which session a dispute of the head goes to. |
| Owner | The runner; written only by `instrukt review start`, in the transaction that captures the start or the send. |
| Loss cost | The requested reviewer resolves no assignment and records no verdict, and a dispute of its head can no longer be asked for: lost intent. |
| Medium | The state database, beside the runtime bindings it is written with. |
| Write isolation | The state database's writer lock. Its only writer is a command the caller waits on, like the runtime bindings it shares a transaction with. |
| Scope | One computer: the requested reviewer runs on this computer's runtime. Moving review evidence between computers moves no ask (`project/instrukt/spec/feature/work/github-apps`, Evidence when the serving computer changes). |
| Lifecycle | One record per head asked for, holding that head's asks. It is current while the canonical branch does not contain the head, and history once it does. History is retired after a retention period, inside `instrukt review start`. |

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
And when that reviewer changes its verdict, the revised verdict is recorded
But when the agent pushes a fixing commit instead
Then the new head is reviewed in its own right, naming the prior report
```

### UC-RQ3: A developer's pull-request head under automatic review is not reviewed twice

```gherkin
Given a developer's project has opted in to automatic review and a pull request's head has a review request in flight
When the developer's agent asks for an independent review of that head
Then no second reviewer is started and the agent is told which review is in flight and where its verdict will show
```

## Known caveats

- Names inside coordinates — a branch name, a file name, a pointer's path — are
  the requester's and reach the reviewer, whose procedure reads them as
  untrusted data (`sdlc/procedure/lifecycle/work/review-commits`).
- A session can still start a reviewer by other means. Such a review is no
  requested review: nothing checks its assignment, and its verdict is admitted
  under the same rules (`project/instrukt/spec/feature/work/github-apps`,
  Admission of review evidence).
- The requested reviewer is a native session. Input another session sends it
  through the runtime is not part of its assignment, and its procedure reads it
  as untrusted data.
- A commit that becomes owed under automatic review while its requested
  reviewer works is reviewed twice. The verdict recorded first settles it; when
  the review request is live first, the requested reviewer's verdict is refused
  and names that request.
- A dispute reaches only the requested reviewer that recorded the verdict.
  Once that session is archived or deleted, or when the verdict was recorded
  by a reviewer started by other means, a dispute is refused and a revised
  head is the way forward.

## See Also

- [project/instrukt/spec/feature/runner/pull-request-review](pull-request-review.md)
- [project/instrukt/spec/feature/runner/session-continuity](session-continuity.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [project/instrukt/spec/feature/configuration/roles-and-capabilities](../configuration/roles-and-capabilities.md)
- [sdlc/procedure/lifecycle/work/review-commits](../../../../domain/sdlc/procedure/lifecycle/work/review-commits.md)
- [general/policy/commits](../../../../general/policy/commits.md)
