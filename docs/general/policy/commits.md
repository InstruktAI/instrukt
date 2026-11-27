---
eval: true
description: 'Worktree-only authoring (`main` is integrator-only), `instrukt repo commit` (Commitizen + provenance), commit-first autonomy, and review routing.'
visibility: public
---

# Commits — Policy

## Rules

Main has one writer: the integrator. Every other agent authors and commits in a worktree, placed by these rules alone:

- **A todo's work** is done by its orchestrator and the workers it dispatches in the todo's single `<slug>/work` worktree, which exists once the orchestrator starts its work: the configured `runtime.start_thread` places it. Pending documentation edits authored there are reviewed alongside implementation and folded by the integrator at delivery.
- **Everything else is ad hoc:** a short-lived one-off worktree from the freshly fetched canonical branch, on a branch whose name carries the change's Commitizen type as its prefix (`docs/…`, `fix/…`), never a todo's branch. Pending documentation written ahead for a todo is ad hoc work: its carriers name the todo, it is committed with `--todo <slug>`, reviewed and merged onto main, and its worktree removed. No todo worktree is opened or kept for it; the todo's own worktree starts from main carrying it.
- **These placements are exclusive.** A todo's work happens only in its worktree, written-ahead documentation only as ad hoc work, and main changes only through the integrator.

Where independent review is owed under the routing rule below, it binds the exact worktree head before integration. Only the integrator merges reviewed heads and publishes main through the existing candidate gate. No per-SHA review note or direct-main exemption path exists.

This is the canonical source of commit behavior and of where repository work is authored. Other docs may reference it, but must not redefine either.

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

Create the todo before its first committing preparation step and associate its worktree commits through `instrukt repo commit --todo <slug>`. Ad hoc work retains its one-off worktree and reviewed head until integration, and removes the worktree and its branch once merged.

**Valid types:**

All Committizen types are allowed.

**Scope:** Component or area affected (optional but recommended)

**Subject:** Clear, concise description in imperative mood

- One logical change per commit
- Keep commits small and focused
- Commit message explains WHY, not WHAT (code shows what)
- A commit that changes directive text explains in its body what no diff shows:
  the problem it answers, the reader role whose behavior it changes and the
  behavior wanted, each requirement that shaped it stated as the requirement
  with its source, the alternatives rejected, and what it leaves as it was
  (`general/policy/documentation-discipline`).
- Update or add tests when behavior changes.
- Include migration notes when schema or data shape changes.
- Squash noisy fixups before merge to keep history readable.
- **Main is green by construction.** The integrator merges a candidate in its
  integration worktree and pushes it to `integrate`; CI runs on that push, and
  only a green result fast-forwards main to exactly that commit. Branch
  protection on main with required status checks refuses any other push. A
  project without CI runs its documented test command on the candidate instead. When main
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
  with `git add` first.
- When the human intends a full commit, first inspect the entire change set,
  then commit the full working tree as-is, with no selective scoping, under a
  clear, meaningful, intent-based commit message.

- Review is routed by the committed worktree diff and binds its head before merge. Functional code/tests are committed in that worktree first. The authoring session
  decides for each commit whether independent review is owed, by what a wrong change would
  cost. Review is owed when the change carries a consequence its author cannot
  establish alone: new or changed logic, a state or data shape, a contract
  between components, concurrency, security, or what a governed snippet
  directs other agents to do. The author dispatches that
  reviewer and waits for the verdict. Every other commit owes no review, and
  the author asks no one: removal of dead code, files, or obsolete snippets, a merge of work already reviewed where it was written, a
  validation or mechanical repair, a rename or move (including snippet relocation),
  documentation curation that leaves rule semantics unchanged and adds nothing to an always-on set (demoting an `index.yaml` entry from `expanded` to `listed`, reordering index entries, or wiring `Required reads` on non-expanded snippets, where `instrukt docs validate` checks structural validity),
  formatting, and wording or
  prompt prose whose effect is presentation. Presentation wording its owner
  judged while it was being written — wording a human shaped with the authoring
  session — is already reviewed where it was written. A change carrying one of
  the consequences above goes to review whoever shaped it, because shaping its
  intent does not establish its effect on the rest of the system. Which paths
  the diff touches never decides that review is owed. Review is technical
  stewardship, never a human ratification step. Removing a live check or branch
  changes logic and owes review. Mechanical changes require green CI only.
  This author-decided routing is listed in the general baseline (loaded on demand or through procedure required reads);
  it adds no checkpoint gate.
  A worktree whose change owes independent review retains the report and approving verdict bound to its exact head before integration. Changes after review require current review; no stamp on an individual direct-main commit substitutes for it.

- A `REQUEST CHANGES` verdict stays with the authoring session for
  evidence-based rejection or in-scope remediation and re-review, applying the
  finding-validation and disposition rules in
  `sdlc/procedure/lifecycle/fix-review-findings`. For commit review, fixing
  commits carry each finding's disposition, independently established basis,
  and closure evidence in their commit bodies; an evidence-only rejection on
  the same head records its dispositions and sources in a disposition file
  beside the prior report. The follow-up message that opens the continuing
  reviewer's revision turn carries only the coordinates (`head`, prior report
  path, and any disposition file path) and no free-text narrative or
  constraints (`sdlc/procedure/lifecycle/work/review-commits`). The author
  retains commit and review-routing responsibilities; the reviewer alone
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
- A full commit the human asks for records the working tree they are looking
  at; scoping it selectively leaves part of their work uncommitted without
  their knowing, and an unread change set gets a message that misstates it.
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
- Carrying directive-change intent and finding dispositions in commit bodies
  (or a named disposition file on an unchanged head) keeps review input
  durable and visibly untrusted, while keeping the reviewer's assignment
  strictly coordinates-only so author framing cannot masquerade as mandate.

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
