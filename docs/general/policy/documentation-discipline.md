---
eval: true
description: 'The enforceable rules of doc-driven development: functionality authors
  its contract first, behavioral deltas stay reflected, and current versus pending
  truth remains explicit.'
visibility: public
---

# Documentation Discipline — Policy

## Rules

### The doc layer tracks reality

- **Every behavioral delta is reflected in the doc layer.** A change that alters
  or adds a documented contract updates the owning snippet, or records an
  explicit `doc_impact_none: <reason>`.
- **A behavioral delta is a change to code-enforced, observable behavior** — a
  contract a test could bind. Descriptive text (what exists, where to look, what
  a flag means) is its own documentation; editing it is a doc-layer change, not a
  contract delta.
- **Directive text on an agent-reachable surface is executable contract.** The
  discriminator is what the words _do_, never who reads them: text that tells its
  reader to take an action — a runtime decision block, a worker brief, a
  procedure step, a CLI message naming a next move — is the implementation.
  `--all runs against every configured source` is descriptive; `run make
restart, then check the logs` is directive. One string reaching a human and an
  agent alike resolves to contract. That no test binds such text makes the change
  higher-risk, not lower. Its remedy is exactly two things: reflect it in the
  owning snippet, and put the commit through independent review. It is never
  labelled copy-only, wording, or "no logic change." A model prompt that shapes
  only how output reads — a summary's voice, a title's style — directs no
  agent's action; it is presentation prose, and its review is decided by
  `general/policy/commits`.
- **A snippet without `delivered_by` is the current contract; staleness is a
  defect.** If code changes and a current snippet does not, the change updates it
  or marks it pending. Pending state and its clearing on delivery are
  `general/principle/documentation-first-development`; the carriers
  are `general/procedure/doc-snippet-authoring`.
- **A limit found is recorded with its contract.** A limit of a current contract
  that investigation or review uncovers is recorded by the session that found it,
  in the owning snippet: a spec's `Known caveats`, or a design's `Failure modes`
  when it is how the system fails. What each section holds is defined in
  `general/spec/snippet-authoring-schema`.

### Doc-driven delivery

- **Todo routing belongs to** `sdlc/policy/todo-creation-discipline`;
  this policy governs the documentation once routing is decided and grants no
  authoring authority.
- **Doc changes to what the human governs are written where the human is.** When
  the human, with the agent they address, decides something that changes a
  process whose outcome is theirs (the closed list in
  `general/policy/human-contract`), that agent writes the doc change itself in the worktree of the session the human is working in,
  before any todo or delegated work that depends on it, and does not hand it to
  another agent. Any other doc change can be written up front, where the
  authoring session holds context a handoff would lose, or delegated, where
  nothing would be lost in translation.
- **Functionality is doc-driven — snippets first.** A todo that delivers
  functionality has its affected snippets authored or updated, pending, before
  its plan is drafted: by the creating session when it holds context a handoff
  would lose, otherwise by the claimant at preparation entry. The planner shapes that doc work into the plan; the
  builder executes against it and does not decide doc reflection on its own.
- **A feature spec is owed only for behavioral work delivered through a todo**,
  against the bounded context that owns the behavior; the classification and
  placement are `sdlc/principle/documentation-first-development`
  and `sdlc/procedure/feature-spec-authoring`. Global doctrine
  (policy, principle, procedure, spec) is directive text: reflected and reviewed
  as above, never given a use case.

### Review and cross-project authoring

- **Governed documentation is never self-reviewed.** A doc-only commit that
  changes what a snippet establishes or directs gets an independent review via
  `sdlc/procedure/lifecycle/work/review-commits` on its worktree head before
  integration. The two review depths are:
  - **A change to what agents are directed to do** — directive text, the
    process layer (`sdlc/policy/process-change-authority`) included — is
    reviewed at full depth. Its intent is carried in the commit body under
    `general/policy/commits`, never in the reviewer's assignment brief.
  - **Any other change to what a snippet establishes** is reviewed at the
    lighter depth for the security lane's repository-privacy check and the
    doc-hygiene lane.

  The author's classification selects the reviewer outside the brief
  (`general/procedure/orchestration`); the reviewer independently determines
  its depth from the diff and never takes classification from the assignment
  (`sdlc/procedure/lifecycle/work/review-commits`).

  Every check is reviewer judgment, never prose scanning. Documentation curation
  and mechanical edits that leave rule semantics unchanged and add nothing to an
  always-on set — demoting an `index.yaml` entry from `expanded` to `listed`,
  reordering index entries, relocating snippets, removing dead or obsolete
  snippets, wiring `Required reads` on non-expanded snippets, typos, formatting,
  and link repairs — change neither what a snippet establishes nor what it
  directs; `instrukt docs validate` checks structural validity (references,
  schema, and closure overlap) and they owe no independent review
  (`general/policy/commits`). Promoting a snippet into an always-on set
  (`expanded`, `agents`, or a `Required read` on an `expanded` snippet) remains
  subject to independent review (`sdlc/policy/definition-of-done-lanes`). Company home knowledge (the documents anywhere under `docs/` in a company
  repository and the `docs/index.yaml` that selects them) is governed
  content: it renders into the company's committed `AGENTS.md` and published
  skills, so its content follows `general/policy/snippet-value-contract` and
  `general/policy/agent-facing-text`, and its changes are authored in a
  separate worktree (keeping the serving company home checkout on the reviewed
  default branch) and put through independent review before reaching the
  default branch (`general/policy/commits`). Standalone descriptive prose
  outside the snippet system and company knowledge, such as a README, is not
  governed content; directive text is, wherever it lives.

- **Cross-project doc work preserves the authoring session.** The session holding
  the design context writes the complete content to caller-owned temporary files
  and relays only their absolute paths and target repository paths to a session
  rooted in the owning project. That agent applies them exactly — no rewriting,
  summarizing, or improving — then runs its project's validation, sync, commit,
  and review routing. If it cannot apply them exactly, it reports the blocker and
  leaves the content unchanged. The caller removes the files once success or
  cancellation is confirmed.

## Rationale

Documentation leads and code catches up
(`general/principle/documentation-first-development`); this policy
is that disposition's rule layer.

Reader identity cannot classify a surface that one string serves to both humans
and agents; what the words do can be read from the string itself, so every
reviewer resolves the same case the same way.

A limit known only to the session that found it is rediscovered by every later
reader, often only after relying on the capability. Recorded with its contract,
it is in front of the reader at the moment of relying on it.

A decision the human makes with one agent reaches every other agent through the
document that agent writes. Handed on as a message instead, it arrives without
the conversation that made it correct, and the human is pulled back to restate
it.

An author reviewing their own snippet believes it correct, which is no review.
A reviewer handed only the diff can judge form, and a directive change can be
well formed while it steers every later session the wrong way. Carrying that
intent in the commit body — checked at commit review against what the human
directed in the producing conversation —
gives both delegated and event-triggered reviewers the same durable, untrusted
basis without letting author prose or author classification enter the
reviewer's mandate. A change that only refines what a snippet establishes steers
no one, so its error costs less and a lighter review catches it. Company home
documents reach every company agent and skill session on merge, so unreviewed
direct commits publish stale history and unverified instructions straight to
every reader.

Moving a mechanical write to another project must not move authorship with it:
the files carry the already-authored document, and the receiving session owns
only application and its project's gates.

## Scope

All agents, all sessions, every documentation change, every domain.

## Enforcement

Plan-review applies the documentation lane as a hard gate; build-review and
worktree-head review apply it as the backstop
(`sdlc/policy/definition-of-done-lanes`). The independent
doc-hygiene worktree-head review catches self-reviewed and stale snippets.

## Exceptions

- Trivial implementation-internal changes with no contract change touch no docs.
