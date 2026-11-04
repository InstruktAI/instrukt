---
description: 'Schema and section requirements for documentation snippets belonging to a taxonomy.'
visibility: 'public'
---

# Snippet Authoring Schema — Spec

## What it is

Defines the required structure, section headings, and directory layout for documentation snippets.

## Canonical fields

A task may carry an optional agent/model dispatch preference. Subscriptions carry event names only, with no home, identity or attendance fields.

Classification (`id`, `type`, `scope`, `domain`) is **derived from the file path** by publication and materialized in the snippet's index entry in the home — it is **never authored in frontmatter**. Validation separately enforces that the H1 type suffix matches the path-derived type. The index is the single source of truth for these four fields:

- `id` — unique snippet identifier, **derived** from the snippet's path below its
  half and, for a project's local snippet, from the project's own declaration.
  There is one identity per snippet, the same from every location:
  - `<domain>/<taxonomy>/<rest>` — a domain snippet, published by the domain's
    owner from its `docs/global/<domain>/`. A project publishes only domain
    snippets.
  - `local/projects/<slug>/<taxonomy>/<rest>` — a snippet in the local half of a
    project.

  An identity that begins with `local/` is never published: its content stays
  in its project, and only its index entry enters the home. Every other
  identity is published. The two halves of a project therefore never share an
  identity. `<slug>` is the slug the project declares in its `instrukt.yaml`
  (`project`), never a directory name. Each index stores its entries under these
  qualified identities; no identity has a shorter or location-relative form.

- `type` — taxonomy type (see table below). **Derived** from the taxonomy directory path and checked against the H1 type-suffix by validation.
- `domain` — **Derived** as the first path segment of the id. It holds either a
  published knowledge domain (`general`, `sdlc`, and their peers) or the
  structural marker `project`; a structural marker is never a published domain.
  A project's local snippet is the exception to the first-segment rule: its
  identity begins with `local/projects`, and its derived domain is `project`,
  never a prefix.
- `scope` — the scope the snippet belongs to: `global` or `project`.
  **Derived** from the half: a domain snippet is `global`, a project's local
  snippet is `project`. More than one project's local snippets can be assembled
  into a single read at once, which is why every local identity names its
  project.

Frontmatter (required, authored):

- `description` — short summary of the snippet's purpose. This is the **only** required authored field.

Frontmatter (optional, authored):

- `visibility` — metadata with no behavior or classification guidance: `public` or `internal`. Defaults to
  `internal` when omitted. It is recorded in the index and filters nothing:
  every reader reads every snippet in full.

- `eval` — optional boolean. `true` marks a snippet whose content enters the
  eval fingerprint: publication computes a digest only of published
  (`docs/global`) snippets with `eval: true`, hashed as committed (raw) content,
  and every stamp carries it.

- `activation_trigger` — procedures only. A non-empty string describing when to activate the procedure. Appears in the "Activatable Procedures" section of its domain's generated bundle, or of the project's generated AGENTS.md for a project's own procedure, for progressive disclosure. Validation rejects this field on non-procedure snippet types.

- `subscriptions` — procedures only. A non-empty list of event type patterns whose arrival activates the procedure through the runner, which resolves subscribed procedures from frontmatter. A pattern is an exact event type name, a prefix of whole name segments followed by `.*` (`a.b.*` selects every event under `a.b`), or `*`; any other form is rejected. A procedure declares `subscriptions` or `activation_trigger`, never both, because each names who is responsible for starting the work; a procedure used only as supporting knowledge declares neither. A procedure declaring `subscriptions` is excluded from the "Activatable Procedures" section and listed as a configured responsibility instead. Validation rejects this field on non-procedure snippet types and on a procedure declaring both activation fields.

- `role` — procedures only. The session role an agent carries when this procedure is its launch root: the procedure a dispatcher or a subscription names. Required on a procedure declaring `subscriptions` and on every procedure a dispatcher names. A procedure may be both a launch root and a dependency or hat another session reads: the entry declarations are interpreted only when the procedure is the launch root, and reading a procedure inside a session interprets none of them and changes nothing about that session. Role is free text in core; with todos enabled it is validated against the roles declared under `roles:` in the selected published
domain’s `workflow/actions.yaml`. Validation rejects the field on any non-procedure snippet.

- **Root frontmatter.** `role`, `subscriptions`, `activation_trigger` and `dispatch` are a procedure's root frontmatter: they mark a procedure a session is started into. A procedure carrying none of them is supporting knowledge, or, inside the group folder an entry procedure heads, that entry procedure's sub-procedure. An entry procedure heads the group folder whose `overview` it is; a variant that requires the shared `overview` of its group heads nothing. Validation reads the frontmatter of every procedure in the validated project and in the published documentation, pending procedures included, and rejects a `role` value two procedures declare, and an entry procedure whose Required reads name a procedure of the group folder it heads that carries no root frontmatter. Each finding names the procedures involved. The checks hold the procedure composition of `general/procedure/doc-snippet-authoring` across deliveries, which no single delivery's review sees (`general/principle/priming`).

- `parameters` — procedures only, beside `role`. An ordered list of the arguments the procedure takes at its launch: each entry has `name`, optional `required` (bool, `false` when omitted: an entry is optional unless it says otherwise), and at most one final entry may carry `rest: true`, taking the whole remaining argument text verbatim. Position is implicit from list order. The launch tokenizes the raw argument text on whitespace, binding one token per positional entry in order and the untokenized remainder to the `rest` entry; a required entry with no token refuses the launch before a session exists; an omitted optional entry is absent; surplus tokens with no `rest` entry refuse the launch. A declaration supplies no value for an argument, so the bound arguments are exactly what the caller supplied and a missing input is never covered by an invented one (`sdlc/principle/design-fundamentals`, Fail Fast). A procedure declaring no `parameters` takes no arguments and refuses any. The loader that starts the session carries a preamble mapping each bound name to its value, and `$ARGUMENTS` names the raw argument text wherever a body refers to it. The entry context an entry type supplies is never a parameter. Validation rejects a duplicate name, a `rest` entry that is not last, an entry key other than `name`, `required` and `rest`, and the field on a snippet that declares no `role`.

- `dispatch` — procedures only, beside `role`. The procedure's launch defaults: `agent`, a non-empty ordered agent preference list, and `thinking_mode`, one of `fast`, `med`, `slow`. Agent names are those the configured runtime mapping can start, not a fixed set or the set receiving deployed instructions. The global `agents.order` supplies the defaults; a procedure declares `dispatch` only where it overrides them. Validation rejects an empty list, an agent name the configured runtime mapping cannot start, an unknown mode, and the field on a snippet that declares no `role`.

- `delivered_by` — list of todo slugs that materialize a **brand-new** snippet's intended state. Optional and non-empty when present; absence means the snippet describes the current platform contract. A snippet carrying `delivered_by` is **pending** and `instrukt docs index` flags it inline with a `(delivered by: <slugs>)` suffix (never hidden; the slug name is coordination metadata, not content). `instrukt docs index --target <slug>` narrows the listing to one todo's new snippets. The pending **body** stays invisible until active, except in the `instrukt docs get <id> --target <slug>` view of a slug that delivers the snippet or carries its own companion or planned material for it. Lifecycle code owns roadmap relationship checks and clears `delivered_by` on delivery (see `sdlc/principle/documentation-first-development` §10); source validation checks frontmatter shape and a todo slug's form (its existence only when the `todos` module is on), and publication records the source state it receives. `delivered_by` is the **only** pending-state frontmatter field. A change to an already-live snippet is **never** marked in frontmatter: its pending material lives in a companion filename (`<stem>.<slug>.md`), an in-body `&lt;!-- planned:slug --&gt;` block, or an in-body `&lt;!-- planned-change:slug --&gt;` block. The index derives the parallel `(superseded by: <slugs>)` suffix on the parent's line from companions, planned blocks, and planned-change blocks, not from any field.

- `date` — `adr` snippets ONLY (required). ISO-8601 date the decision was recorded.
- `number` — `adr` snippets ONLY (required). Integer matching the filename prefix (e.g., `0007` for `0007-provenance-via-mirror-id.md`).

Classification (`id`/`type`/`scope`/`domain`) is read from the file path by publication, never authored in frontmatter. H1 type-suffix correctness is enforced by validation.

Third-party docs under `docs/third-party/` or `~/.instrukt/docs/third-party/` are indexed separately from taxonomy snippets. They author only `description`; their `id` and `scope` are derived from their path below the third-party root, and they no longer carry `type`.

### Event delivery declarations

A procedure declaring `subscriptions` may also declare:

- `event_delivery`: `sequential` (default) or `batched`. This selects one oldest
  pending event or the ordered pending set at each eligible delivery boundary.
- `interruptible`: a boolean, default `false`. False admits activation events
  only at initial activation and wake entry; true also admits them at supported
  in-turn hook boundaries.

These fields are valid only on subscribed procedures. Their values are
independent: cardinality changes how many events are supplied, while
interruptibility changes when they may arrive. The runtime enforces the declared
delivery timing and cardinality.

Delivery settings configure a procedure session at initialization and remain
fixed for that session, including revival. Newly initialized successor sessions
use the current declaration. Publication changes do not reconfigure a running
agent beneath its assignment (`general/principle/trust-and-guardrails`).

A subscribed procedure explicitly declares its applicable shared event-handling
instructions in Required reads. Publication and retrieval resolve those authored
references through the ordinary dependency graph; delivery metadata supplies
runtime settings rather than injecting documentation dependencies. Shared
handling duties are conditional on the actual invocation and its fixed settings,
including when the procedure is read inline
(`general/policy/referencing-doc-snippets`, `general/principle/priming`).

Document structure:

- H1 title with type suffix (e.g., `# Session Lifecycle — Design`).
- `## Required reads` — immediately after H1, with inline `@` references for hard dependencies. These are expanded (inlined) at build time.
- Required H2 sections per taxonomy (see table below).
- Additional structure inside valid sections is allowed. Use subordinate headings such as `###` when they improve clarity; schema validation constrains the required H2 section names, not the internal organization within them.
- `## Sources` — third-party docs may include web links or a documentation index service's snippet IDs when authoritative references are available.
- `## See Also` — optional soft references (not inlined at build time).
- `<!-- @include: docs/snippets/<name>.md -->` — inlines a shared fragment's body exactly where the directive sits, in every rendering of the snippet: `instrukt docs get`, a session's assignment brief and generated bundles. It carries text several snippets state word for word, so it is written once. A fragment is authored under `docs/global/snippets/` and published as `docs/snippets/`; it has no frontmatter, is not a snippet and is never retrieved by ID. A fragment that cannot be found leaves its directive visible in the rendering.
- `<!-- @config: <path> -->` — replaced by the configured value at `<path>` when the snippet is read, resolved in-process through configuration; in compiled outputs (bundles, `AGENTS.md`, global entrypoints) it is resolved at publication, where a `runtime.*` path is a finding. The format has no directive that runs a command: `@exec` is a validation finding and is never run.

### Required H2 sections per taxonomy

| Taxonomy  | Required sections                                                           |
| --------- | --------------------------------------------------------------------------- |
| principle | Principle, Rationale, Implications, Tensions                                |
| concept   | What, Why                                                                   |
| policy    | Rules, Rationale, Scope, Enforcement, Exceptions                            |
| procedure | Goal, Preconditions, Steps, Outputs, Recovery                               |
| design    | Purpose, Inputs/Outputs, Invariants, Primary flows, Failure modes           |
| spec      | What it is, Canonical fields (optional: Allowed values, Known caveats)      |
| adr       | Context, Decision, Consequences (optional: Alternatives Considered, Status) |

`Discipline` is the only additional procedure H2. It is optional unless an
agent command explicitly delegates to that procedure's discipline; such a
delegation requires the procedure to define it. The section is the procedure's
behavioral anchor: it states the act the procedure's agent holds to at the
moment of acting, where the work is most likely to drift, as the reality
wanted from it (`general/policy/agent-facing-text`). An anchor another
procedure could carry equally is not a Discipline, and its measure is
specificity, never length.

`Known caveats` states the limits of a spec's current contract that a reader
would otherwise assume away: what the surface deliberately does not do or
support, what it does not recover from, and a capability built without a
consumer in what ships alongside it. Each caveat is current behavior, true
until a change to code or shipped configuration moves it. What a capability may
later be for is intent and lives in a todo; behavior a delivery will add is
pending material. How a designed system fails belongs in a design snippet's
`Failure modes`.

Supporting material stays subordinate to the required section that owns it:

- `### Examples` and `### Pre-completion checklist` belong inside `## Steps`.
- `### Report format` belongs inside `## Outputs`.

Examples demonstrate representative invocation patterns. They are
non-exhaustive and never define an allowlist; mandatory commands and prohibited
alternatives belong in the applicable numbered step or governing policy.

Design snippets should include Mermaid diagrams where visual representation aids understanding. Use flowcharts for data flow, sequence diagrams for interactions, and class diagrams for structural relationships.

### Directory layout

Every repository has the same two halves:

- `docs/global/` — the **published** half. Publication copies it into
  `~/.instrukt/docs/`, where it is served to every reader.
- `docs/local/` — the **local** half. It is never published: its content stays
  in the project, its index entries are patched into the home, and each entry
  carries the absolute path of the file in the project, from which it is read.

The half decides where a snippet is served from and whether its identity
carries the `local/` prefix. It never decides who may read a snippet
(`general/policy/artifact-scope`).

- `docs/global/{domain}/baseline.md` and `docs/global/{domain}/baseline-progressive.md` — a published domain's baseline manifests (curated required reads), generated into that domain's bundle; `general` is a domain like any other.
- `docs/global/{domain}/{taxonomy}/` — a published domain's documentation (e.g., `general`, `sdlc`).
- `docs/local/{taxonomy}/` — project-scoped documentation.

- A repository's `docs/local/{taxonomy}/` snippets derive the identity
  `local/projects/{project}/{taxonomy}/{rest}`.
- The same relative path under both halves of a repository yields two snippets with two
  identities.
- `docs/local/spec/domain/{context}.md` — bounded-context domain spec (one per service).
- `{half}/spec/feature/{context}/{concept-name}.md` — feature spec (concept-named, hub in the hub-and-spoke model per `sdlc/principle/documentation-first-development`), under the half whose bounded context owns the behavior; `docs/local/` is the common one, not the only one. Contains Gherkin acceptance scenarios + business value. Many-to-many with todos via `delivered_by`.
- `docs/local/design/architecture/<topic>/overview.md` — canonical design snippet for a composite architecture topic. Named sibling design snippets carry independently useful facets; topic-owned non-Markdown companions may sit beside them.
- `<stem>.<slug>.md` (sibling to any `<stem>.md`) — a **companion**: the complete next version of `<stem>.md`, where `<slug>` is the delivering todo. A non-empty companion is a full snippet that shares the parent's derived `id` (the validator detects it by the sibling filename and exempts the id↔path / id-collision checks). An **empty** (zero-byte) companion is a **retirement**: the parent has no next version, the validator accepts it without snippet checks, and `instrukt docs get <id> --target <slug>` renders an empty pending view. The link lives entirely in the filename, the index never lists the companion, and default readers see the parent. On delivery a non-empty companion **replaces** the parent file and is deleted; an empty companion is deleted together with the parent. Bounded additions use in-body `&lt;!-- planned:slug --&gt;` blocks; bounded replacements and removals use in-body `&lt;!-- planned-change:slug --&gt;` blocks. All three live-parent carriers contribute to the parent's `(superseded by: <slugs>)` index suffix.
- `docs/local/adr/{NNNN}-{kebab-case-title}.md` — an `adr` snippet (type derived from the `adr/` path). Architectural decision records. Capture what was chosen and why, so specs/procedures/principles can describe the current state without becoming history logs. `date` and `number` are required additional frontmatter fields. ADRs sit at top-level under `docs/local/` (peer to `spec/`, `design/`, `policy/`, `procedure/`, `concept/`) — they are reasoning records, not design exploration, and do not belong under `design/`. An `adr` snippet carries no `delivered_by`, no `supersedes`, and no companion file — ADRs sit outside the pending/delivery machinery and do not cross-reference each other via frontmatter. The ADR body references earlier ADRs only (a backward in-prose reference); the one forward link is the optional `## Status` section's supersession pointer (`Superseded by ADR-NNNN` / `Superseded in part by ADR-NNNN` / `Deprecated`), added to an existing ADR when a later one supersedes it. `## Status` is the only part of an ADR that may change after authoring; the body is immutable.

- **In-body marker grammar.** The `planned`, `planned-change`, and `change` markers are **whole-line**: each occupies its own line, with only whitespace before its opening `<!--` and after its closing `-->`. Markers **never nest** — an in-body block holds only plain content between its own markers, never another `planned`/`planned-change` block, and two carriers never overlap. Marker validation rejects inline (mid-prose) and nested markers as malformed, so every accepted marker is one the delivery fold can resolve.

- `docs/local/baseline.md` — project baseline manifest (curated required reads).

Published snippets are distributed to `~/.instrukt/docs/` under their identity. Local snippets stay in their project and are served through their index entries in the home; the default index view lists the local snippets of the project the caller stands in, and a reader reaches another project's by naming that project or a snippet's `local/` identity.

### Grouped snippets — folder convention

When multiple snippets share a topic (e.g., a general procedure plus thin-wrapper variants, or a design family with several facets), they live together in a folder named for the topic. The folder contains:

- `overview.md` — the top-level or shared snippet for the group (e.g., the general procedure that variants build on).
- Sibling files for specialized variants or facets, named for what they cover (e.g., `human.md`, `project.md`, `prepare.md`, `work.md`).

The folder name becomes part of every member snippet's id (e.g., `general/procedure/event-handling/overview`, `general/procedure/event-handling/interruptible`). Variants reference the overview as a required read. All members share the same taxonomy type — a folder under `procedure/` contains only `procedure` snippets.

Apply this convention whenever a group of related snippets would otherwise produce a flat sibling list with shared-prefix filenames (e.g., `foo-overview.md`, `foo-human.md`, `foo-project.md` becomes `foo/overview.md`, `foo/human.md`, `foo/project.md`). The flat-prefix anti-pattern is rejected at review.

Non-Markdown source and rendered assets owned exclusively by a grouped topic
may live beside its snippets. They are companions, not snippet members: they do
not receive taxonomy metadata, cannot replace the group's `overview.md`, and do
not relax the rule that all Markdown members share one taxonomy type. A
composite architecture topic may therefore contain `overview.md`, named design
facets, `canvas.excalidraw`, and an optional consumed `canvas.svg`. Keep an SVG
only when documentation or another durable surface uses it; generated renders
do not accumulate by default.

## Allowed values

- `type`: principle, concept, policy, procedure, design, spec, adr.
- `scope`: global, project.
- Identity prefixes: `projects` and the local-half prefix `local`. They are
  reserved: no published domain carries one of these names.

- `visibility`: public, internal. Default: `internal`.

## Known caveats

- Baseline manifests contain only `@` references and do not use frontmatter or H1 sections.
- Section headings must match the schema exactly; extra headings are invalid.
- For schema validation, "section headings" means the required H2 taxonomy sections. Subordinate headings inside a valid section are allowed.

- The global index and every project's local index are written into the home by publication, each project patching only its own entries — never hand-edit them. No index file lives in a repository.
