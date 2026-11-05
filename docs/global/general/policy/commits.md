---
eval: true
description: Commitizen format, attribution, when to commit. Atomic commits, hooks
  verify quality.
visibility: public
---

# Commits — Policy

## Rules

Direct commits land on local main. Only the integrator pushes main, carrying settled direct commits through `integrate`, CI and exact-commit fast-forward; there is no CI-exempt route. A direct-commit review records its verdict and report digest with `instrukt repo stamp <sha…> --verdict approve|needs_work --report <file>` as a git note on each commit. Notes travel with push and need no separate work record; the integrator reads them.

This is the canonical source of commit behavior. Other docs may reference it, but must not redefine commit policy.

Write a Commitizen subject and, when useful, an explanation of the change:

```
type(scope): subject
```

- Author only the subject and explanatory body. The sealed commit interface
  owns attribution and provenance. The message describes the work, not who
  performed or authorized it (`general/concept/lineage-vs-authorship`).
- Declare human involvement through the command's action-scoped option under
  `general/policy/human-involvement-recording`.

**Todo association:**

For documentation prepared for a future todo, choose the fitting slug before
committing and supply it through `instrukt repo commit --todo <slug>`. Choosing the
slug creates and reserves no todo. Work associated with an existing todo uses
the same flag. Work outside a todo keeps its ordinary commit path.

**Valid types:**

All Committizen types are allowed.

**Scope:** Component or area affected (optional but recommended)

**Subject:** Clear, concise description in imperative mood

- One logical change per commit
- Keep commits small and focused
- Commit message explains WHY, not WHAT (code shows what)
- Update or add tests when behavior changes.
- Include migration notes when schema or data shape changes.
- Squash noisy fixups before merge to keep history readable.
- **Main is green by construction.** The integrator merges a candidate in its
  integration worktree and pushes it to `integrate`; CI runs on that push, and
  only a green result fast-forwards main to exactly that commit. Branch
  protection on main with required status checks refuses any other push. A
  project without CI runs its `test` action on the candidate instead. When main
  still turns red, it is reverted first and repaired in its own todo.
- **Commit authorized, coherent work first; then route review.** Committing is
  part of normal autonomous agent work once the current authoring context has
  produced a stable change it is authorized to materialize. A dispatched worker
  or an agent-owned autonomous session does not ask the human for routine commit
  permission.
- **Automation reports commit state; it never creates commit authority.** A
  checkpoint, stop hook, or tool result may require attention
  under its governing contract, but it does not turn a forming human
  conversation into a finished change. Where the human has not released the
  tracked change for commit, report what remains uncommitted and offer to
  commit; do not commit it, revert it, or suppress the automation.
- Commit through `instrukt repo commit`. It stages nothing: stage the change
  with git first, or name its paths with `--only <path>...` to commit just
  those.

- Post-commit review is routed by the committed diff, not by pre-commit
  permission. Functional code/tests are committed first. The authoring session
  decides for each commit whether independent review is owed, by what a wrong change would
  cost. Review is owed when the change carries a consequence its author cannot
  establish alone: new or changed logic, a state or data shape, a contract
  between components, concurrency, security, or what a governed snippet
  directs other agents to do. The author dispatches that
  reviewer and waits for the verdict. Every other commit owes no review, and
  the author asks no one: a removal, a merge of work already reviewed where it was written, a
  validation or mechanical repair, a rename or move, formatting, and wording or
  prompt prose whose effect is presentation. Presentation wording its owner
  judged while it was being written — wording a human shaped with the authoring
  session — is already reviewed where it was written. A change carrying one of
  the consequences above goes to review whoever shaped it, because shaping its
  intent does not establish its effect on the rest of the system. Which paths
  the diff touches never decides that review is owed. Review is technical
  stewardship, never a human ratification step.
  For a commit in the own-stamp class that owes no independent review, its
  author records `instrukt repo stamp <sha> --verdict exempt --reason <text>`.
  Direct commits settle with either an independent `approve` note and report
  binding or an author's `exempt` note and reason; `needs_work` settles neither.

- A `REQUEST CHANGES` verdict stays with the authoring session for
  evidence-based rejection or in-scope remediation and re-review. Before
  remediation, the author records each finding's disposition and its
  independently established basis in the follow-up to the continuing reviewer,
  applying the finding-validation and disposition rules in
  `sdlc/procedure/lifecycle/fix-review-findings`. After remediation, the
  follow-up identifies the fixing commits and closure evidence; an
  evidence-only rejection names its sources. For commit review, this exchange
  carries the disposition record. The author retains commit and review-routing responsibilities; the reviewer alone
  determines the verdict. What cannot proceed — a genuine human-owned decision,
  a safety gate, or a disagreement author and reviewer cannot resolve — is
  held, and the session goes idle with it; nothing routes it to another party
  (`general/principle/autonomy`).
  The authoring session owns acting on its review verdict — remediation,
  re-review routing and recovery; the verdict itself is the reviewer's.

## Rationale

The committing agent owns the explanation of the change; the platform owns its
verified attribution and provenance. Operating guidance supplies the command
arguments that carry the author's intent, rather than metadata representations
that invite the author to imitate platform output (`general/principle/priming`).

- Consistent commits enable reliable rollbacks and audits.
- Commit messages drive automated release notes and changelogs.
- Asking before routine commits shifts technical stewardship back to the human
  and leaves completed work unrecorded. The safety boundary is post-commit
  review routing, not a human permission prompt.
- A review is worth what it can catch. It earns its cost where a wrong change
  does damage its author cannot see and an outside standard can find it. A
  change whose error shows at a glance and undoes in a moment, or whose only
  standard is the judgment its owner already applied, gives the reviewer
  nothing to check: the review spends a session and the human's attention and
  verifies nothing. Deciding by what the commit touches instead of what it
  risks turns every edit to a code file into a review cycle.

## Scope

- Applies to all repositories and all contributors.

## Enforcement

`instrukt repo commit` is the canonical commit interface. It composes the
commit's attribution and provenance — the agent as git author, `Session:`,
`Human-Involved:` from the caller's `--human-involved`, `Todo:`, `Planned-Todo:` — and runs no
gate of its own: it does not stage, and it does not block raw git.

The sealed interface refuses caller-supplied platform metadata before committing.
Its refusal directs the caller to submit a subject and explanation through the
supported command arguments. Reviewers judge authored content and command use;
generated provenance defects belong to the platform implementation.

Commit guidance, CLI help, agent-specific policies, and project-specific designs must not
require a pre-commit human decision for autonomously authorized work. They may reference this policy and document
mechanics, but the commit-first rule and the human-conversation boundary live
here.

## Exceptions

- Emergency production fixes may allow minimal commits with a follow-up cleanup task.
