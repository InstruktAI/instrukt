---
description: An authoring session asks for the independent review of an exact committed worktree head by naming its coordinates only, and the reviewer's assignment is rendered by instrukt from them.
delivered_by:
  - fixed-shape-commit-review-dispatch
---
# Requested Review — Spec

## What it is

Committed work that instrukt asks no review for on its own initiative — a
one-off worktree head, a human-directed branch — is reviewed when its authoring
session asks (`general/policy/commits`). A requested review is that ask, made
through instrukt. The requester names where the candidate is; instrukt checks
the candidate and renders the reviewer's assignment itself. No words of the
requester's reach the reviewer, so the reviewer's mandate is its procedure
(`sdlc/procedure/lifecycle/work/review-commits`) and the candidate, as it is
when instrukt asks for a pull request's review
(`project/instrukt/spec/feature/runner/pull-request-review`).

## Canonical fields

### Coordinates

A requested review is asked for with coordinates. Each names a place or a
commit and carries no text of its own.

| Coordinate | Names | Present |
| --- | --- | --- |
| Worktree | The checkout that holds the candidate. | Always |
| Branch | The branch the candidate is on. | Always |
| Head | The exact candidate commit, by its full id. | Always |
| Base | The pinned base, the commit the reviewed change starts from, by its full id. | Always |
| Report destination | Where the reviewer writes its complete report. | Always |
| Prior report | The report of the earlier review. | On re-review |
| Disposition file | The author's dispositions of the earlier findings, with their sources (`general/policy/commits`). | On re-review of an unchanged head |
| Source pointer | A source the reviewer reads beside the candidate: a snippet id, or a file at a commit of a repository, as `repo@commit:path`. Any number of them. | Optional |

The ask has no part that takes narrative, a classification, a constraint, a
summary or an instruction. What an author wants a reviewer to know travels in
the commit bodies of the reviewed range or in the disposition file
(`general/policy/commits`), and the reviewer reads both as its procedure
directs. A source pointer adds to what the reviewer reads and removes nothing
from it (`sdlc/procedure/lifecycle/work/review-commits`).

The pinned base of a requested review is the head's merge base with the
project's canonical branch, as its review procedure pins a one-off candidate
(`sdlc/procedure/lifecycle/work/review-commits`).

### Admission

A requested review is admitted only when all of these hold. They are facts of
the candidate, read from the repository when the review is asked for.

| Check | Rule | The refusal names |
| --- | --- | --- |
| Form | Every value has the form of its coordinate: a full commit id, a branch name, a path, a snippet id, or a `repo@commit:path` pointer. | The value and the form its coordinate takes. |
| Placement | The worktree is a checkout of the project's repository, the named branch is checked out in it, and that branch's head is the named head. | What the repository shows instead. |
| Cleanliness | The worktree holds no uncommitted change. | That the worktree is not clean. The door is committing or removing the change. |
| Range | The head is a commit of the repository that shares history with the project's canonical branch, and the named base is the pinned base. | The pinned base, or that the head shares no history with the canonical branch. |

A refused ask starts no reviewer and records nothing.

### The reviewer's assignment

instrukt renders the assignment from the admitted coordinates in one fixed
wording: the procedure the reviewer executes, and the coordinates. The same
coordinates always render the same assignment, and it holds nothing the
requester composed. The reviewer of a requested review is started on that
rendering alone; the ask offers no way to add to it.

A requested review is answered by the procedure that answers a review instrukt
asks for on its own initiative, under the role that procedure declares
(`sdlc/procedure/lifecycle/work/review-commits`), and covers the commits after
the pinned base up to the exact head. The reviewer is independent of the
candidate's authors as its procedure and the admission of its verdict require
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence).

### Verdict

Asking for a review records no verdict and no review evidence. The reviewer
records its verdict with `instrukt review record`, admitted under the rules
that bind every head verdict (`project/instrukt/spec/feature/work/github-apps`,
Admission of review evidence), and returns its outcome as its procedure
directs.

### Re-review

A revised candidate is a new head. Its review is asked for as that head's, and
names the prior report. A dispute of findings without a new commit names the
unchanged head, the prior report and the disposition file; which session judges
it follows from the settlement of that head's verdict
(`project/instrukt/spec/feature/work/github-apps`, Admission of review
evidence). Both carry coordinates only.

## Use cases

### UC-RQ1: A developer's committed work is reviewed from its coordinates alone

```gherkin
Given a developer's agent has committed work on a branch in a clean worktree of the developer's project
When the agent asks for an independent review of that exact head
Then an independent reviewer is started on an assignment that names where the work is and holds nothing the agent wrote
But when the worktree holds uncommitted work or the branch is no longer at the named head
Then the review is not started and the agent is told what differs and how to repair it
```

## Known caveats

- Names inside coordinates — a branch name, a file name, a pointer's path — are
  the requester's and reach the reviewer, whose procedure reads them as
  untrusted data (`sdlc/procedure/lifecycle/work/review-commits`).
- A session can still start a reviewer by other means. Such a review is no
  requested review: nothing checks its assignment, and its verdict is admitted
  under the same rules (`project/instrukt/spec/feature/work/github-apps`,
  Admission of review evidence).

## See Also

- [project/instrukt/spec/feature/runner/pull-request-review](pull-request-review.md)
- [project/instrukt/spec/feature/work/github-apps](../work/github-apps.md)
- [sdlc/procedure/lifecycle/work/review-commits](../../../../domain/sdlc/procedure/lifecycle/work/review-commits.md)
- [general/policy/commits](../../../../general/policy/commits.md)
