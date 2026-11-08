---
description: 'Where snippets live and how they are named: the three positions, ids, domains, index.yaml and schema.yaml, frontmatter including event subscription home, identity and attendance, and the reference forms. Section shape comes from instrukt docs schema.'
visibility: 'public'
---

# Snippet Authoring Schema — Spec

## What it is

Defines where documentation snippets live and how they are identified, the `index.yaml` and `schema.yaml` files that select and shape them, their frontmatter, and the reference forms. The sections a snippet carries at a folder are printed by `instrukt docs schema <folder>`.

## Canonical fields

Feature specs may carry `binding_reservations: {UC-id: todo-slug}`. Each entry owns one scenario binding independently of pending visibility. Preparation folding preserves these until attached tests deliver; work-delivery folding clears only bound entries.

### Positions and identity

A repository holds documentation in up to three positions:

| Position            | Reaches agents                                                                                       | Identity                         |
| ------------------- | ---------------------------------------------------------------------------------------------------- | -------------------------------- |
| `docs/general/`     | always on: rendered into every agent CLI's user-scope instruction file                               | `general/<taxonomy>/<rest>`      |
| `docs/domain/<x>/`  | on demand: one line in each global agent file; rendered in full by `instrukt docs get --domain <x>` | `<x>/<taxonomy>/<rest>`          |
| `docs/project/`     | the project: rendered into its root `AGENTS.md`                                                      | `project/<slug>/<taxonomy>/<rest>` |

<!-- planned-change:instrukt-first-build -->
General and domain snippets are published from the repository's committed
main. Project snippets are indexed from committed main, but retrieval reads
their files in the registered primary checkout; source edits affect that read.
<!-- change:instrukt-first-build -->
Every position is published from the repository's committed main. Project
snippets are copied to `~/.instrukt/docs/project/<slug>/…` and read from that
store, like general and domain snippets. The source checkout remains the
authoring location; uncommitted edits are not retrieval content.
<!-- /planned-change:instrukt-first-build -->

`<slug>` is the slug the project declares in its
`instrukt.yaml` (`project`), never a directory name. An id is a name, not a
copy of the path: a domain snippet's id carries no `domain/` prefix.

Each published unit — `docs/general/` and each domain — has one owner per
computer. A second repository claiming a unit another repository already owns
is refused, and the refusal names the current owner.

Classification (`id`, `type`, `scope`, `domain`) is **derived from the file
path** by publication and materialized in the snippet's index entry — it is
**never authored in frontmatter**. Validation separately enforces that the H1
type suffix matches the path-derived type.

- `id` — unique snippet identifier, derived from the position and the path below
  it as the table shows. There is one identity per snippet, the same from every
  location, and no identity has a shorter or location-relative form.
- `type` — taxonomy type. **Derived** from the taxonomy directory path and
  checked against the H1 type suffix by validation.
- `domain` — **derived** as the id's first segment: `general`, a domain name,
  or `project`.
- `scope` — `global` for a snippet under `docs/general/` or `docs/domain/*/`,
  `project` for a snippet under `docs/project/`. More than one project's
  snippets can be assembled into a single read at once, which is why every
  project identity names its project.

### Domains

A domain is a body of knowledge one publisher is the authority on, published as
one unit: one owner per computer, its own `schema.yaml`, its own `index.yaml`,
and optionally its own workflow (`docs/domain/<x>/workflow/`). A domain is named
after what it is about, never after a category it belongs to. Domains carry no
tags; an agent finds a domain through its `index.yaml` `description` and its
procedures' activation triggers.

### `index.yaml`

Each position carries at most one `index.yaml`, at its root. It selects what
the position's rendering carries; depth below that comes from required reads.

| Position                     | Allowed keys                        |
| ---------------------------- | ----------------------------------- |
| `docs/general/index.yaml`    | `expanded`, `listed`, `agents`      |
| `docs/domain/<x>/index.yaml` | `description`, `expanded`, `listed` |
| `docs/project/index.yaml`    | `expanded`, `listed`                |

- `expanded` — snippet ids rendered in full, each with its closure: its
  transitive required reads, each once.
- `listed` — snippet ids rendered as one line each: id, description and the
  command that gets it.
- `description` — the domain's one line in every global agent file, which
  names `instrukt docs get --domain <x>`.
- `agents` — a map from a supported agent CLI name to snippet ids. Those
  snippets are ordinary `policy` snippets, appended to that one CLI's global
  file only.
- `general`'s `expanded` and `listed` may name ids from any domain, which is
  how always-on content from several domains is chosen in one place. A
  domain's index and the project's index name only snippets of their own
  position.

`instrukt docs validate`, and the sync that publishes the position, report as
findings a key the position does not allow (`agents` outside
`docs/general/index.yaml` among them), an id that names no delivered snippet,
an `agents` key that is not a supported agent CLI, a `listed` id already in the
closure of `expanded`, and an `index.yaml` anywhere but a position's root; sync
refuses on them. A domain without `index.yaml` is a notice: it is
absent from the global file and found only through `instrukt docs index`.

What each rendering carries — the global agent files, `instrukt docs get
--domain <x>`, the project's root `AGENTS.md` — is owned by
`general/policy/agent-artifact-governance`.

### `schema.yaml` and section shape

The seven types, their required and optional sections, and the generic rules
(H1, frontmatter, Required reads, See Also) are fixed in instrukt. Each position
may carry one `schema.yaml` at its root, keyed by a folder path relative to that
root, declaring additional required and optional H2 sections for snippets under
that path. A path adds sections and never drops one a shorter path or the root
rules require. Inheritance never crosses positions or repositories, so a
snippet validates the same way on every computer. A project without
`schema.yaml` follows the root rules only.

`instrukt docs schema <folder>` prints the effective shape at a folder: per
type, the required and optional sections, the file and path key imposing each
rule, and a skeleton. Validation findings name the same file and key.

### Frontmatter

A task may carry an optional agent/model dispatch preference. A subscribed procedure declares event type patterns and its subscription home, with optional identity and attendance settings as defined under Event subscription declarations.

Frontmatter (required, authored):

- `description` — short summary of the snippet's purpose. This is the **only** required authored field.

Frontmatter (optional, authored):

- `visibility` — metadata with no behavior or classification guidance: `public` or `internal`. Defaults to
  `internal` when omitted. It is recorded in the index and filters nothing:
  every reader reads every snippet in full.

- `eval` — optional boolean. `true` marks a snippet whose content enters the
  eval fingerprint: publication computes a digest only of published
  snippets (under `docs/general/` and `docs/domain/*/`) with `eval: true`, hashed as committed (raw) content,
  and every stamp carries it.

- `activation_trigger` — procedures only. A non-empty string describing when to activate the procedure. The procedure is listed with its id, description, trigger and the command `instrukt docs get <id> --closure`: a `general` procedure in `general`'s section of every global agent file, a domain's procedure under that domain's line in every global agent file and in `instrukt docs get --domain <x>`, and a project's procedure in the project's root `AGENTS.md`. When a procedure earns a trigger is decided by `general/procedure/doc-snippet-authoring`. Validation rejects this field on non-procedure snippet types.

- `subscriptions` — procedures only. A non-empty list of event type patterns whose arrival activates the procedure through the runner, which resolves subscribed procedures from frontmatter. A pattern is an exact event type name, a prefix of whole name segments followed by `.*` (`a.b.*` selects every event under `a.b`), or `*`; any other form is rejected. A procedure declares `subscriptions` or `activation_trigger`, never both, because each names who is responsible for starting the work; a procedure used only as supporting knowledge declares neither. A procedure declaring `subscriptions` is never listed among triggered procedures. Validation rejects this field on non-procedure snippet types and on a procedure declaring both activation fields.

<!-- planned-change:instrukt-first-build -->
- `role` — procedures only. The session role an agent carries when this procedure is its launch root: the procedure a dispatcher or a subscription names. Required on a procedure declaring `subscriptions` and on every procedure a dispatcher names. A procedure may be both a launch root and a dependency or hat another session reads: the entry declarations are interpreted only when the procedure is the launch root, and reading a procedure inside a session interprets none of them and changes nothing about that session. Role is free text in core; with todos enabled it is validated against the roles declared under `roles:` in the selected published domain's `workflow/actions.yaml`. Validation rejects the field on any non-procedure snippet.
<!-- change:instrukt-first-build -->
- `role` — procedures only. The session role an agent carries when this procedure is its launch root: the procedure a dispatcher or a subscription names. Required on every procedure a workflow dispatcher names. A subscribed procedure may omit it and receives the effective role `event-handler`. An unknown configured mapping uses `default` without losing the explicit role name. A procedure may be both a launch root and a dependency or hat another session reads: the entry declarations are interpreted only when the procedure is the launch root, and reading a procedure inside a session interprets none of them and changes nothing about that session. Role supplies the single key for class, scoped capability bundles, model selection and optional GitHub App identity in `~/.instrukt/agents.yaml`. A reading of supporting knowledge changes none of these assignment facts. Workflow role-list membership is checked only for procedures that workflow dispatches; subscribed procedures keep their declared role independently of todos. Validation rejects the field on any non-procedure snippet.
<!-- /planned-change:instrukt-first-build -->

- **Root frontmatter.** `role`, `subscriptions` and `activation_trigger` are a procedure's root frontmatter: they mark a procedure a session is started into. A procedure carrying none of them is supporting knowledge, or, inside the group folder an entry procedure heads, that entry procedure's sub-procedure. An entry procedure heads the group folder whose `overview` it is; a variant that requires the shared `overview` of its group heads nothing. Validation reads the frontmatter of every procedure in the validated project and in the published documentation, pending procedures included, and rejects a `role` value two procedures declare, and an entry procedure whose Required reads name a procedure of the group folder it heads that carries no root frontmatter. Each finding names the procedures involved. The checks hold the procedure composition of `general/procedure/doc-snippet-authoring` across deliveries, which no single delivery's review sees (`general/principle/priming`).

<!-- planned-change:instrukt-first-build -->
- `parameters` — procedures only, beside `role`. An ordered list of the arguments the procedure takes at its launch: each entry has `name`, optional `required` (bool, `false` when omitted: an entry is optional unless it says otherwise), and at most one final entry may carry `rest: true`, taking the whole remaining argument text verbatim. Position is implicit from list order. The launch tokenizes the raw argument text on whitespace, binding one token per positional entry in order and the untokenized remainder to the `rest` entry; a required entry with no token refuses the launch before a session exists; an omitted optional entry is absent; surplus tokens with no `rest` entry refuse the launch. A declaration supplies no value for an argument, so the bound arguments are exactly what the caller supplied and a missing input is never covered by an invented one (`sdlc/principle/design-fundamentals`, Fail Fast). A procedure declaring no `parameters` takes no arguments and refuses any. The loader that starts the session carries a preamble mapping each bound name to its value, and `$ARGUMENTS` names the raw argument text wherever a body refers to it. The entry context an entry type supplies is never a parameter. Validation rejects a duplicate name, a `rest` entry that is not last, an entry key other than `name`, `required` and `rest`, and the field on a snippet that declares no `role`.
<!-- change:instrukt-first-build -->
- `parameters` — procedures only, beside `role`. An ordered list of the arguments the procedure takes at its launch: each entry has `name`, optional `required` (bool, `false` when omitted: an entry is optional unless it says otherwise), and at most one final entry may carry `rest: true`, taking the whole remaining argument text verbatim. Position is implicit from list order. The launch tokenizes the raw argument text on whitespace, binding one token per positional entry in order and the untokenized remainder to the `rest` entry; a required entry with no token refuses the launch before a session exists; an omitted optional entry is absent; surplus tokens with no `rest` entry refuse the launch. A declaration supplies no value for an argument, so the bound arguments are exactly what the caller supplied and a missing input is never covered by an invented one (`sdlc/principle/design-fundamentals`, Fail Fast). A procedure declaring no `parameters` takes no arguments and refuses any. The loader that starts the session carries a preamble mapping each bound name to its value, and `$ARGUMENTS` names the raw argument text wherever a body refers to it. The entry context an entry type supplies is never a parameter. Validation rejects a duplicate name, a `rest` entry that is not last, an entry key other than `name`, `required` and `rest`, and the field on a snippet with neither an explicit `role` nor subscriptions supplying the effective `event-handler` role.
<!-- /planned-change:instrukt-first-build -->

- `delivered_by` — list of todo slugs that materialize a **brand-new** snippet's intended state. Optional and non-empty when present; absence means the snippet describes the current platform contract. A snippet carrying `delivered_by` is **pending** and `instrukt docs index` flags it inline with a `(delivered by: <slugs>)` suffix (never hidden; the slug name is coordination metadata, not content). `instrukt docs index --target <slug>` narrows the listing to one todo's new snippets. The pending **body** stays invisible until active, except in the `instrukt docs get <id> --target <slug>` view of a slug that delivers the snippet or carries its own companion or planned material for it. Lifecycle code owns roadmap relationship checks; the integrator clears `delivered_by` when reviewed preparation integrates, before build (see `sdlc/principle/documentation-first-development`); source validation checks frontmatter shape and a todo slug's form (its existence only when the `todos` module is on), and publication records the source state it receives. `delivered_by` is the **only** pending-state frontmatter field. A change to an already-live snippet is **never** marked in frontmatter: its pending material lives in a companion filename (`<stem>.<slug>.md`), an in-body `&lt;!-- planned:slug --&gt;` block, or an in-body `&lt;!-- planned-change:slug --&gt;` block. The index derives the parallel `(superseded by: <slugs>)` suffix on the parent's line from companions, planned blocks, and planned-change blocks, not from any field.

- `date` — `adr` snippets ONLY (required). ISO-8601 date the decision was recorded.
- `number` — `adr` snippets ONLY (required). Integer matching the filename prefix (e.g., `0007` for `0007-provenance-via-mirror-id.md`).

Third-party docs under `docs/third-party/` or `~/.instrukt/docs/third-party/` are indexed separately from taxonomy snippets. They author only `description`; their `id` and `scope` are derived from their path below the third-party root, and they no longer carry `type`.

### Event subscription declarations

A subscribed procedure declares `subscription_home`: `project`, `platform`,
`company` or `person`. Optional `subscription_identity` is `home` (default) or
`subject`, and `subscription_attendance` is `attended` (default) or `unattended`.
These are procedure-only and require subscriptions; malformed or unknown values
are findings. A project-position procedure declares project home and answers only
its own project; general/domain publication reaches all homes of its declared
kind. Company/person vocabulary does not supply executable home management.
Validation/publication own interpretation; routing forwards the resolved facts
unchanged. Reading a procedure as supporting knowledge reconfigures no session.

### Event delivery declarations

A procedure declaring `subscriptions` may also declare:

- `interruptible`: a boolean, default `true`. True requests as-soon-as-possible
  delivery: steer at the next safe point where supported, otherwise hand over
  queued input now or use the earliest idle entry. False requests only-at-entry
  delivery, allowing the current run to finish undisturbed. The shipped
  integrator and upstream-watch explicitly declare false.

This field is valid only on subscribed procedures. Delivery is always a bounded
batch; the intent determines when its entries may arrive. Missing steering is a supported
runtime degradation, never a declaration finding. The adapter realizes the
intent with the runtime's capabilities; pending wakes and messages combine,
while distinct event identities and bounded batch selection remain.
On a queue-only runtime at most one instrukt message waits per session; later
input remains pending for the next combined message.

Delivery settings configure a procedure session at initialization and remain
fixed for that session, including continuation after human settling of a standing
thread. Newly initialized successor sessions
use the current declaration. Publication changes do not reconfigure a running
agent beneath its assignment (`general/principle/trust-and-guardrails`).

A subscribed procedure explicitly declares its applicable shared event-handling
instructions in Required reads. Publication and retrieval resolve those authored
references through the ordinary dependency graph; delivery metadata supplies
runtime settings rather than injecting documentation dependencies. Shared
handling duties are conditional on the actual invocation and its fixed settings,
including when the procedure is read inline
(`general/policy/referencing-doc-snippets`, `general/principle/priming`).

### Document structure

The sections each type requires and allows at a folder are printed by
`instrukt docs schema <folder>` (see `schema.yaml` and section shape above);
this spec does not restate them. Across every type:

- H1 title with type suffix (e.g., `# Session Lifecycle — Design`).
- `## Required reads` — immediately after the H1, one reference per list item.
  A reference names one snippet id, or a bare domain name (`- sdlc`), which
  stands for that domain's `expanded` set. A same-repository snippet reference
  is `- [<id>](<relative path>)`: the text is exactly the id and the target is
  relative to the referring file, naming that id's file in the working tree.
  A reference to another repository stays `- <id>`. Resolution and hygiene use
  only the id; validation accepts plain ids too and checks each link's text,
  existence, repository confinement and match to that id's file. Agent
  rendering strips the links to `- <id>`. A snippet path outside these
  reference entries remains a reference-by-path finding.
  `instrukt docs get <id>` names a snippet's required reads, and `--closure`
  renders them with it.
- Additional structure inside valid sections is allowed. Use subordinate headings such as `###` when they improve clarity; validation constrains the H2 section names, not the internal organization within them.
- `## Sources` — third-party docs may include web links or a documentation index service's snippet IDs when authoritative references are available.
- `## See Also` — optional soft references by id, using the same link form for
  same-repository snippets, never rendered into a closure. A code reference stays a repository-relative path.
- `<!-- @config: <path> -->` — replaced by the configured value at `<path>` when the snippet is read, resolved in-process through configuration. In compiled outputs — the global agent files and a project's `AGENTS.md` — it is resolved at publication against the publishing project's effective configuration, where a `runtime.*` path is a finding. The format has no directive that runs a command or includes another file: `@exec` and `@include` are validation findings, never run or expanded. Text several snippets need is a snippet of its own, named as a required read.

A procedure's optional `Discipline` section is required once an agent command
explicitly delegates to that procedure's discipline. The section is the procedure's
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

The position decides where a snippet is served from and how its identity reads.
It never decides who may read a snippet (`general/policy/artifact-scope`).

- `{position}/{taxonomy}/` — a position's documentation: `docs/general/{taxonomy}/`, `docs/domain/{domain}/{taxonomy}/`, `docs/project/{taxonomy}/`.
- `{position}/index.yaml` and `{position}/schema.yaml` — the position's index and section schema, one each at its root, never in a subfolder.
- `docs/domain/{domain}/workflow/` — a domain's workflow, as its workflow spec defines it.
- The same relative path under two positions of a repository yields two snippets with two identities.
- `docs/project/spec/domain/{context}.md` — bounded-context domain spec (one per context).
- `{position}/spec/feature/{context}/{concept-name}.md` — feature spec (concept-named, hub in the hub-and-spoke model per `sdlc/principle/documentation-first-development`), under the position whose bounded context owns the behavior; `docs/project/` is the common one, not the only one. Contains Gherkin acceptance scenarios + business value. Many-to-many with todos via `delivered_by`.
- `docs/project/design/architecture/<topic>/overview.md` — canonical design snippet for a composite architecture topic. Named sibling design snippets carry independently useful facets; topic-owned non-Markdown companions may sit beside them.
- `<stem>.<slug>.md` (sibling to any `<stem>.md`) — a **companion**: the complete next version of `<stem>.md`, where `<slug>` is the delivering todo. A non-empty companion is a full snippet that shares the parent's derived `id` (the validator detects it by the sibling filename and exempts the id↔path / id-collision checks). An **empty** (zero-byte) companion is a **retirement**: the parent has no next version, the validator accepts it without snippet checks, and `instrukt docs get <id> --target <slug>` renders an empty pending view. The link lives entirely in the filename, the index never lists the companion, and default readers see the parent. On delivery a non-empty companion **replaces** the parent file and is deleted; an empty companion is deleted together with the parent. Bounded additions use in-body `&lt;!-- planned:slug --&gt;` blocks; bounded replacements and removals use in-body `&lt;!-- planned-change:slug --&gt;` blocks. All three live-parent carriers contribute to the parent's `(superseded by: <slugs>)` index suffix.
- `docs/project/adr/{NNNN}-{kebab-case-title}.md` — an `adr` snippet (type derived from the `adr/` path). Architectural decision records. Capture what was chosen and why, so specs/procedures/principles can describe the current state without becoming history logs. `date` and `number` are required additional frontmatter fields. ADRs sit at top-level under `docs/project/` (peer to `spec/`, `design/`, `policy/`, `procedure/`, `concept/`) — they are reasoning records, not design exploration, and do not belong under `design/`. An `adr` snippet carries no `delivered_by`, no `supersedes`, and no companion file — ADRs sit outside the pending/delivery machinery and do not cross-reference each other via frontmatter. The ADR body references earlier ADRs only (a backward in-prose reference); the one forward link is the optional `## Status` section's supersession pointer (`Superseded by ADR-NNNN` / `Superseded in part by ADR-NNNN` / `Deprecated`), added to an existing ADR when a later one supersedes it. `## Status` is the only part of an ADR that may change after authoring; the body is immutable.

- **In-body marker grammar.** The `planned`, `planned-change`, and `change` markers are **whole-line**: each occupies its own line, with only whitespace before its opening `<!--` and after its closing `-->`. Markers **never nest** — an in-body block holds only plain content between its own markers, never another `planned`/`planned-change` block, and two carriers never overlap. Marker validation rejects inline (mid-prose) and nested markers as malformed, so every accepted marker is one the delivery fold can resolve.

The default index view lists the published snippets and the project snippets of the project the caller stands in; a reader reaches another project's by naming that project or a snippet's `project/<slug>/` identity.

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
- Domain names: any name except `general` and `project`, which are reserved,
  and `third-party`, which keeps its existing special case; a domain claiming
  one is refused, naming the rule.
- `agents` keys in `docs/general/index.yaml`: the supported agent CLIs —
  `claude`, `codex`, `gemini`, `deepseek`.
- `visibility`: public, internal. Default: `internal`.

## Known caveats

- Validation accepts only the H2 sections the effective schema names; subordinate headings inside a valid section are free.
- The generated documentation index lives in instrukt's state, written by publication, each project patching only its own entries — never hand-edit it. The authored `index.yaml` files are a different thing: sources a position's owner writes.
