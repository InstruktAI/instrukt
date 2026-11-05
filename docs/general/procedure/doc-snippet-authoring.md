---
activation_trigger: Use when creating, changing, proposing, or drafting a documentation
  snippet first enters consideration — before its shape is decided.
description: Establish whether documentation work is ready and authorized, then author
  snippets according to the taxonomy, schema, and policies.
visibility: public
---

# Snippet Authoring — Procedure

## Required reads

- @~/.instrukt/docs/general/concept/documentation-snippets.md
- @~/.instrukt/docs/general/policy/artifact-scope.md
- @~/.instrukt/docs/general/policy/agent-facing-text.md
- @~/.instrukt/docs/general/principle/intent-first-documentation.md
- @~/.instrukt/docs/sdlc/principle/documentation-first-development.md
- @~/.instrukt/docs/general/policy/referencing-doc-snippets.md
- @~/.instrukt/docs/general/policy/snippet-value-contract.md
- @~/.instrukt/docs/general/spec/snippet-authoring-schema.md

## Goal

Establish authority and convergence before materializing documentation, then
create or update a snippet that conforms to schema and is discoverable by
tooling.

## Preconditions

- Snippet taxonomy and schema references available.
- Target position and folder determined (step 1).
- Authoring authority established in Step 0.

## Steps

Authoring reads and edits the snippet's source file in the repository. `instrukt docs get` is for knowledge: it renders the snippet, filters pending content and resolves references, so its output is not the file you are changing. Before committing, run `instrukt docs validate` on the authored docs. Sync refuses committed docs with findings and retains the previous publication; no pre-commit hook is installed.

0.  **Establish authoring authority, then earn the snippet.** Load this procedure
    when a snippet first enters consideration, before drafting its shape. A
    bounded dispatched brief or an original explicit authoring request supplies
    authority for its scope. When the current human conversation began as
    exploration or design, authoring starts only after every active participant
    agrees the outcome and the human releases it for materialization. Silence,
    unilateral confidence, partial agreement, and automated checkpoint or
    approval output supply none. Until the gate opens, keep forming work in
    conversation or a non-authoritative scratchpad and stop before editing
    Git-tracked documentation on shared `main`.

    Once authorized, name the specific future-agent question this snippet
    answers and confirm it is not already answered by a one-minute source read,
    an already-loaded global policy or principle, the project's README, or an
    existing snippet. If you cannot name the question, or any of those already
    answers it, do not write the snippet — either skip it, or add a one-line
    entry under "See also" of an existing snippet pointing to the upstream
    source. Apply `general/policy/snippet-value-contract` rigorously: the default
    is **skip unless justified**, not "produce when applicable."

1.  Place the file at the correct path — the path determines `id`/`type`/`scope`/`domain` (publication derives and materializes them; you do not author them). Choose the position first (`general/policy/artifact-scope`): `docs/general/` for what every agent on every computer needs, `docs/domain/<x>/` for the body of knowledge one domain is the authority on, `docs/project/` for this project's own knowledge. Then choose the folder within the position whose overview the snippet belongs under. A new domain is named after what it is about, never after a category it belongs to, and never `general` or `project` (`general/spec/snippet-authoring-schema`). In a project whose domain specs follow `sdlc/spec/domain-spec`, a new design snippet is listed, in the same change, in the domain spec of the context it governs or in the context map when it is cross-cutting.
2.  Author top-down: broader context should point to narrower details, not the other way around. Name the role of the reader this snippet's load path reaches, and write the reality wanted from that role (`general/policy/agent-facing-text`).

3.  Add frontmatter: `description` (required) and `visibility`; `id`/`type`/`scope`/`domain` are derived from the path and must NOT be authored. `visibility` has no behavior or classification guidance. Repository-local identity is not embedded: internal snippets also use semantic placeholders for computer, user, host, path, and topology values, then direct readers to the runtime discovery surface. Repository privacy is reviewed semantically; never create or request a prose scanner for it.

4.  Write H1 title with correct type suffix.
5.  **Required reads follow the folder.** Require the overview of the folder
    the snippet sits in, when the folder has one and the snippet is not it. Add a further read only when it is specific to this
    snippet and not already in the overview's closure, naming it by id
    (`general/policy/referencing-doc-snippets`). What every snippet in an area
    needs goes into that area's overview, never into each snippet. When a
    reader would need "the overview minus a few", split the overview into the
    parts readers need separately and require the parts that apply; never
    subtract.
6.  Run `instrukt docs schema <folder>` for the target folder and author the
    sections it names for the snippet's type, starting from its skeleton.
7.  Organize content inside those valid sections. Use subordinate headings such as `###` when they improve readability; do not flatten useful structure just to avoid internal headings.
8.  If validation rejects a heading, first ask whether it should be downgraded under an existing schema section rather than deleted or flattened.

9.  Authoring a snippet publishes nothing. Readers receive committed
    documentation: the git hooks publish the committed state of the project's
    main after a commit, merge, rebase or checkout, and `instrukt sync`, run in
    the project, republishes it. No path publishes a working tree. This step supplies neither authoring nor commit
    authority.
10. **Decide what the position's `index.yaml` carries.** A snippet reaches
    agents through `expanded`, `listed` or `agents` in its position's
    `index.yaml`, or through another snippet's required reads; one with none of
    these is found only through `instrukt docs index`. Every `expanded` entry,
    with its whole closure, and every `listed` line is read by every agent the
    position reaches — for `docs/general/`, every agent on every adopting
    computer — and an `agents` entry by every session of that CLI, before any
    task begins (`general/policy/snippet-value-contract`, Manual in the
    always-on set). Put a snippet in `expanded` only when every one of those
    agents acts on it in ordinary work; put it in `listed` when they need to
    know it exists and fetch it when it applies; otherwise leave it out and
    let the overview or the procedure that needs it require it. Keep a `listed`
    id out of the closure of `expanded`. An `agents` entry is a correction for
    one agent CLI's observed default, written as a `policy` snippet. Never
    edit the generated documentation index; publication writes it.

11. **Give a procedure an `activation_trigger` only when a user will ask for it
    by name.** Every trigger is a line in every agent's context on every
    computer that adopts its domain (`general/policy/snippet-value-contract`,
    Manual in the always-on set). A procedure a workflow, a dispatcher or
    another procedure starts is reached through that starter and carries no
    trigger.
12. **Changing an existing snippet — bounded, wholesale, or retired.** A live snippet is never overwritten or deleted in place to describe a change before it is delivered. Match the pending carrier to the shape of the change: bounded additions, replacements, and removals stay in body markers; wholesale reworks use a companion; retiring the whole snippet uses an empty companion. The pending carrier names the delivering `<slug>`.

            - **Present-tense body (governs both forms).** Author the wrapped or companion body as the **future live contract**, in present tense, exactly as it must read once delivered. On delivery the marker lines vanish (ADD) or the companion replaces the parent (REPLACE) and the body stays **verbatim** — so it must already read as correct current-state contract with nothing stripped. The delivering `<slug>` is carried by the planned marker or the companion filename. A slug names work and a snippet body states contract, so **no slug appears in body prose**. Do not write "Pending `<slug>`, …", "the new surface is …", "will", "is planned", or any history/rationale narration inside the body (see the Narration anti-pattern in `general/policy/snippet-value-contract`). DON'T: `Pending <slug>, the manual-parking surface is: …`. DO: `The manual-parking surface is: …` — wrapped in `<!-- planned:<slug> -->`, which is the only place the slug appears.
            - **Change-shape heuristic.** A bounded localized edit uses an in-body marker: `planned` for added content, `planned-change` for replacement or removal. A wholesale rewrite uses a companion file. Retiring the whole snippet uses an empty companion file. ADRs do not use pending markers or companions; they land as immutable decision records.

            - **Markers occupy their own line.** Every `planned`, `planned-change`, and `change` marker sits alone on its line — only whitespace before it and after it. Inline (mid-prose) markers are rejected by the marker validator. To replace a clause inside a sentence, wrap the whole containing line or paragraph as the current/replacement spans rather than embedding markers in the prose; the rendered result is identical and the fold stays unambiguous.


            - **Markers sit between whole Markdown blocks.** Place every marker so adding or resolving its lines changes no block around it, and the snippet is formatted and checked like any other file (`sdlc/policy/linting-requirements`). A marker between the rows of a table splits the table: to add, change, or remove a row, wrap the whole table.


            - **Markers never nest.** No marker may open inside another slug's open block — one in-body block contains only plain content between its own markers, never another `planned` / `planned-change` block. When a pending change targets a region a different slug already wraps in a pending marker, dependency-order the todos (`instrukt todo update B --after A`) so the later change authors against the earlier slug's approved design — never nest the two in flight. This is the in-body counterpart of the companion **Ordering** rule. The marker validator rejects nested markers, including markers authored inside a fenced code block — a live marker is validated wherever it sits. Sample marker syntax is exempted by placeholder slug, not by position: write it with `<slug>` or `{slug}` so it is not read as a live marker.
            - **Touching another slug's pending block.** Decide first what your edit is. **Aligning** it — the owning slug's intent moved and its pending text no longer matches — is an edit to _their_ contract, not a claim on it: make it in place inside their markers, add no marker of your own, and say so where review will see it — the `log` artifact when the work has one, the commit message when it does not. **Superseding** it — your delivery makes their text false — must be attributable to you and must not land before you do: carry it in a region you own, or wait for their fold and declare the deferral in your plan's documentation lane. Nesting is what supersession would require, so a supersession that can only live inside their block waits.
            - **ADD** (new content: a new Gherkin scenario, a new rule, a new section). Author it **in context under its correct heading**, wrapped in a planned marker that names the slug:

              ```
              <!-- planned:<slug> -->
              ...the new content...
              <!-- /planned:<slug> -->
              ```

              Gherkin appends as more `#### UC-…` blocks; never reshuffle. On delivery the two marker lines are removed and the content stays exactly where it is, now live.

            - **CHANGE / REMOVE** (bounded replacement or localized removal). Author the current text and its replacement in place, wrapped in a planned-change marker that names the slug:

              ```
              <!-- planned-change:<slug> -->
              ...the current content...
              <!-- change:<slug> -->
              ...the replacement content...
              <!-- /planned-change:<slug> -->
              ```

              An empty replacement section is a localized removal. Default `instrukt docs get <id>` renders the current content with markers stripped. `instrukt docs get <id> --target <slug>` and delivery reconciliation render the replacement content with markers stripped.

              **The current content is live contract and stays true.** A pending
              carrier holds what a delivery will improve; it never holds what is
              already false. When the rework was prompted by finding that live
              text is wrong rather than merely improvable, correct the current
              side in the same pass and let the replacement describe the future.

            - **REPLACE** (a wholesale rework). Author a **companion file** `<stem>.<slug>.md` as a sibling of the parent `<stem>.md` — the **complete next version** of the parent, not a diff: a full snippet with the parent's authored frontmatter fields and a full body. The link is **the filename alone** — do **not** add any frontmatter field to the parent. The index derives a `(superseded by: <slug>)` suffix on the parent's line from the companion's existence, so reading the index shows a replacement is in flight; the companion itself never gets its own index line. On delivery the companion's content **replaces** the parent file and the companion is deleted (handled by the integration machine). The validator detects the companion by its sibling filename and validates it as a full snippet, exempting only id-collision checks because it deliberately derives the same id as the parent.

            - **RETIRE** (the delivery removes the capability the whole snippet describes). Author an **empty companion** `<stem>.<slug>.md` beside the parent — zero bytes, because the parent's next version is nothing. The parent stays live and true until the delivery lands, so the live contract never runs ahead of the code (`general/policy/documentation-discipline`). On delivery the fold deletes the parent and the companion together. Every reference to the retiring snippet from another snippet — Required reads, See Also, inline citations — is removed by the same slug's own `planned-change` carriers, so nothing points at the deleted file once it folds. A parent that carries another slug's pending markers retires only after that slug resolves: dependency-order the retiring todo after it (`instrukt todo update B --after A`), because deletion cannot keep the other slug's material.

            - **Ordering.** Two pending companions on the same parent MUST be dependency-ordered (`instrukt todo update B --after A`) — never two independent reworks of one file in flight. Delivery is dependency-gated, so they merge-and-delete in order, each authored against the post-merge parent.

        - **Discoverability.** `instrukt docs index` flags pending work inline, never hiding it: `(delivered by: <slugs>)` on a brand-new snippet not yet live, `(superseded by: <slugs>)` on a live parent with a pending companion, planned section, or planned-change section (derived from the filename/marker). Companions never get their own line. The pending **bodies** stay invisible until active: default `instrukt docs get <id>` strips planned additions and renders the current side of planned-change blocks; `instrukt docs get <id> --target <slug>` reveals that slug's pending view. `instrukt docs index --target <slug>` narrows the listing to one todo's new snippets.

        - **Related-file discovery.** Before editing a live parent `<stem>.md`, list sibling files by stripping `.md` and globbing the stem (`ls docs/.../<stem>.*`). If a non-empty pending companion `<stem>.<slug>.md` exists, mirror the parent edit into that companion or coordinate ownership. A parent edit that leaves its non-empty pending companion unsynced is a defect. An empty companion is a retirement and is never mirrored into: content in it makes it a replacement.

13. **Every rule reaches its reason in one hop.** A normative rule you author is
    traceable to its reason — and the reason names the invariant protected or the
    failure prevented, never a restatement of the rule. Which route is available
    depends on the taxonomy, and every type resolves: `principle` and `policy`
    carry the reason in `Rationale`, or inline in one clause; `concept` carries it
    in `Why`, which serves the same purpose under another name; an `adr` carries it
    in its own decision-record sections. `procedure`, `spec`, and `design` have no
    reasoning-bearing section at all, and for those the only route is a cited
    principle, policy, or ADR, because
    `general/policy/snippet-value-contract` drops rationale from a body whose type
    provides no home for it. Reaching for an inline clause in those types is the
    common way this rule and that one are made to collide. This is what lets a future reader apply the rule to a novel case, contest
    it with evidence, or retire it when its reason has died; a rule that can reach
    no reason is a deletion candidate, not a mystery to preserve. Reviewers treat a
    new why-less rule as a finding; the existing corpus is retrofitted
    progressively as rules are touched, never as a campaign.

14. **ADR discipline (decision capture).** Specs, procedures, and principles describe the _current state_; they hold no reasoning logs. An ADR is reserved for a real, contestable decision and the default is none — consult `general/policy/adr-discipline` for the bar before writing one.

15. **Every edit meets the value contract, not only a new snippet.** Step 0
    decides whether a snippet should exist; this step governs every region you
    write, including a pending block inside an existing snippet. Before
    committing, hold each changed region against the anti-patterns in
    `general/policy/snippet-value-contract`: a value, path, filename, example,
    rule, or scenario whose owner is code, configuration, a schema, or another
    snippet is replaced by the owner's name or id; a region that re-observes
    another context's decision through this snippet's surface is consumer
    restatement and is not written.

16. **Compose procedures by standing role.** A procedure a session is started
    into carries root frontmatter — `role`, `subscriptions`,
    `activation_trigger` or `dispatch` (`general/spec/snippet-authoring-schema`).
    Before authoring one, name the standing role whose responsibility the new
    work is. When a role already carries it, the new event or trigger becomes a
    route in that role's entry procedure: the entry procedure declares every
    event its role answers, routes each one to the step that handles it, and
    is the only procedure declaring that role. When no role carries it, the new
    entry procedure declares a role of its own.

    Handling too large to stay a step in the entry procedure becomes a
    sub-procedure: a procedure in the group folder the entry procedure heads,
    carrying none of the root frontmatter. The entry procedure's route instructs
    its agent to load the sub-procedure with `instrukt docs get <id>` when that
    route is taken, and never lists it in Required reads, which are inlined into
    every activation. Shared handling that other procedures use as well, such as
    the event-handling instructions, stays a Required read. One agent then holds
    the focus and context its role's events share, and the handling of an event
    enters its awareness only when that event arrives
    (`general/principle/priming`).

## Outputs

- New or updated snippet with correct structure.

## Recovery

- If `instrukt docs validate` reports findings, fix them and rerun it.
