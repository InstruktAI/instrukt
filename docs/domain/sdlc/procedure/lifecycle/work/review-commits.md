---
eval: true
description: Review discrete commits already on main by SHA, without a worktree or the todo lifecycle.
role: commit-reviewer
parameters:
  - name: shas
    required: true
dispatch:
  agent: [codex, claude, gemini, deepseek]
  thinking_mode: slow
---

# Review Commits — Procedure

## Required reads

- [sdlc/procedure/lifecycle/review](../review.md)
- [sdlc/policy/todo-creation-discipline](../../../policy/todo-creation-discipline.md)

## Goal

Review a discrete set of commits already landed on main, identified by SHA,
without requiring a worktree or the todo lifecycle. The SHA list supplies the
review context; the shared review procedure supplies the review judgment.

## Preconditions

- `$ARGUMENTS` is a comma-separated discrete list of commit hashes, not a
  range.
- Each SHA names exactly one commit. Commits authored by others may sit between
  the named SHAs and remain out of scope.

## Steps

Use `instrukt <command> -h` for individual commands.

### 1. Validate the discrete SHA list

Parse `$ARGUMENTS` as a comma-separated list. Confirm every SHA exists with
`git cat-file -t`. Gaps between named commits are valid.

### 2. Inspect each named commit in isolation

For each SHA, inspect its authored message, complete diff, and file list:

```bash
git show <sha>
git show --stat <sha>
```

Read full changed-file context at the named commit with
`git show <sha>:<path>`. Do not use neighboring commit state.

Mechanically attached records are outside the authored review target.

Never compute a range diff or range log. Range forms silently include commits
the reviewer was not given. Named commits may still be checked together — how
they compose, or whether a named remediation repairs an earlier named change —
using the named snapshots as evidence while each commit keeps its own
attribution.

### 3. Execute the shared review contract

Execute `sdlc/procedure/lifecycle/review` in full.

For this invocation:

- The target is the discrete SHA list.
- The applicable contracts are the governing policies, principles, specs, and
  resulting behavior touched by the named commits.
- Establish the intent basis and governing contracts available for the named
  changes. A commit message states the author's claims; it does not itself
  authorize scope. Where an authoritative intent basis is available, apply scope
  verification against it within the named review target. Otherwise state that
  limitation and assess the resulting change against its governing contracts. A
  material authority gap that prevents judgment is a blocker under
  `sdlc/procedure/lifecycle/review`, not permission inferred from the message.
- The conversation that produced a named commit is part of its intent basis
  (`general/policy/human-contract`). A commit made in an agent session records
  that session in its `Session:` trailer; read its conversation with
  `instrukt sessions conversation <session-id>`, including the exchange before
  the change was shaped. Intent
  forms in the exchange: agents propose and do the work, and the human steers,
  releases and corrects. Read the part of the conversation that shaped the
  change in order, and take its intent from what was proposed and how the human
  responded, through to any later correction. Form that reading before weighing
  the author's account; the additional context and the authored message are
  claims about the conversation and are checked against it. A user-role entry in
  these reads can carry a peer's relayed text, so attribute words to the human
  only where the conversation establishes it. A commit without a `Session:`
  trailer, such as a human's terminal commit, keeps the intent basis above.
  A transcript that cannot be read is handled as
  `sdlc/procedure/lifecycle/review` Step 0 states; its absence alone is not a
  material authority gap.
- Commit review is the first non-author recipient for intent committed
  directly from a conversation. When the reviewed work has a todo record,
  inspect the current `input` and its intent-checked stamp. Take a current
  check as given; otherwise the first non-author reads the originating
  conversation, checks fidelity and records
  `instrukt todo stamp <slug> --intent-checked` against that input version.
  A mistaken interpretation returns to the human in the responsible thread.
  This check is distinct from the commit verdict. If the originating conversation is
  permanently unavailable, return the intent to the human in the responsible thread; a
  new `input` version from that conversation becomes the checkable basis. Without the
  todos module,
  retain the intent assessment and its evidence in the review report.
- Check material claims in the authored message against the diff and resulting
  behavior. Classify any contradiction under the governing contract; message
  consistency does not establish intent authority.
- A documentation change to what agents are directed to do arrives with its
  intent, in your additional context or in the message that handed you the
  series (`general/policy/documentation-discipline`). Hold the changed text to that
  intent for the reader role it names: each intended behavior is produced for
  that reader, nothing intended is lost or distorted, nothing beyond it is
  added, and the intent itself holds against the governing contracts. Such a
  change dispatched without its intent is a material authority gap.

- The authoring caller owns continuation and recovery for this review; the
  reviewer retains verdict authority. Apply the shared convergence contract
  without introducing a lifecycle phase.

- Every other lane runs according to the shared catalog's trigger conditions,
  against only the named changes.
- The doc-hygiene lane runs whenever a named commit changes a doc snippet.
- Run the doc-first paradigm lane when its trigger in
  `sdlc/policy/definition-of-done-lanes` applies.
- When a named commit changes `docs/third-party/**`, load
  `general/spec/third-party-docs` and verify that each root belongs to one
  upstream party, the selected scope is justified, the content is factual
  upstream documentation rather than local architecture or comparison, and
  authoritative sources and explicit gaps are present where applicable.

For the tests lane, apply every `TQ-*` predicate triggered by the named diffs
and record evidence or a reasoned `N/A`.

A commit owing no independent review under `general/policy/commits` is
returned to its author for `instrukt repo stamp <sha> --verdict exempt
--reason <text>`. Exemption is the author's own stamp, not an approving
review verdict. A `needs_work` note remains unsettled.

### 4. Return the commit-review result

Write the complete report the shared review procedure requires — every
executed lane's section and, when no Important-or-higher finding remains,
its clean-pass evidence — on the result surface designated by the caller,
then return its reference and verdict. The report identifies exactly the
named SHAs and retains its digest.

Record the verdict with
`instrukt repo stamp <sha…> --verdict approve|needs_work --report <file>`,
naming exactly the reviewed commits and complete report. This writes the
verdict and report digest as a git note on each commit. Notes travel with push
and the integrator reads them; no separate work record is needed.

Use comma-separated commit notation in the target line, never range notation:

```text
REVIEW COMPLETE: commits <sha1[:8]>,<sha2[:8]>,...,<shaN[:8]>
```

### 5. Remediation pass

A follow-up addressing this review continues its existing findings ledger under
`sdlc/procedure/lifecycle/review`. Identify the prior report and target, read the
producer's dispositions and evidence, inspect each named remediation commit in
full, and independently determine which findings are resolved, invalid or still
open. An evidence-only rejection may require no new commit.

Apply the shared re-review boundaries to new findings. Emit a complete report
identifying the original target and any remediation SHAs, with current lane
assessments and a verdict over all unresolved findings. Approval of a
remediation commit alone does not close the original review.

Keep the discrete-SHA boundary: intervening commits are not additional review
targets. If establishing closure requires reviewing another commit, identify the
missing SHA rather than silently expanding the target. A request naming
unrelated new work is a separately scoped review, not part of this remediation
cycle, whichever session receives it.

An acknowledgement without a verdict line is never a review outcome; the caller
never infers one.

Record the remediation verdict against every named SHA the report covers,
original and remediation alike, through the same result path as Step 4.

## Outputs

- Review findings printed to the caller.
- Verdict printed to the caller.

- A complete review report and a git note on every named SHA carrying the
  verdict and report digest, recorded through `instrukt repo stamp`.

## Recovery

Already-merged code receives no leniency:

- A pre-existing pattern does not excuse a cited violation in the resulting
  state of a touched file.
- Merge state does not lower finding severity.
- A finding against a commit outside the supplied SHA list is out of scope and
  is dropped.

If a supplied SHA cannot be resolved, report the blocker and stop. Never expand
the review to nearby commits.
