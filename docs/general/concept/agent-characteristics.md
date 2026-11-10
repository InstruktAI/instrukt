---
description: Baseline agent CLI profiles and the reasoning for ordered agent preferences.
visibility: public
---

# Agent Characteristics — Concept

## What

These observations inform role-mapping judgment. Model choice is configured
by role, with a default for unlisted roles, independently of procedure content.
Profiles identify existing runtime provider instances and typed per-tier model
chains. Procedures supply their role rather than model-selection settings.
Runtime startable profiles are distinct from the agent CLIs receiving deployed
instructions.

### Selection paradigm

Availability comes first, then performance, then cost. An unavailable agent
cannot do the work; among available agents, capability for the work determines
the preference, with API-billed CLIs at the end. The strongest available agent
is the default parent and orchestrator; the next is its fallback. Claude is
preferred as that parent and orchestrator. Codex is the other frontier model
and its fallback.

Automation follows the declared order mechanically, taking the first agent
that is available; how availability is established is the configuration's and
the runner's concern. Composing and maintaining the order is judgment: the
profiles inform that judgment, rather than supplying a scoring algorithm.

The shipped role map preserves a cross-model pattern: authors prefer Claude,
reviewers prefer Codex. Defaults cover every harness T3 builds in, including
capable inexpensive and open-model routes. Orders are heuristics the user owns;
DeepSeek has no special fallback position in instrukt's defaults.

### Profiles

These profiles preserve observations from sustained collaboration. A CLI can
expose several models: its name alone does not establish the capability or cost
of every model behind it. Subscription access and API billing describe the
configured access route, not an immutable property of the CLI. Where a source
does not establish strengths, weaknesses or billing, that gap remains explicit.

**Claude / Claude Code** (Anthropic)

- Strong at: architecture, oversight, review, preparation, general-purpose
  reasoning, system design, policy interpretation, codebase navigation and
  multi-step orchestration.
- Weak at: frontend/UI coding and creative visual work.
- Register: analytical, structured, converges toward coherence.
- Cost class: subscription when used through a subscription; API-billed when
  used through an API key.
- Typical order: first; the default parent and orchestrator.

**Codex** (OpenAI)

- Strong at: backend work, thorough coverage, meticulous implementation,
  exhaustive analysis, contract integrity, alternative viewpoints and edge
  case discovery. Preferred for root-cause diagnosis: exhaustive path-walking
  and skepticism help trace symptoms past the first plausible explanation.
- Weak at: may over-engineer and can be rigid about structure.
- Register: methodical, skeptical, surfaces what others miss.
- Cost class: subscription through ChatGPT plan access; other login routes
  require checking the configured billing rather than assuming plan coverage.
- Typical order: the other frontier model; second and fallback to Claude.

**Gemini** (Google)

- Strong at: modern greenfield backend work, frontend/UI, creative work,
  analysis, building, visual thinking, rapid prototyping and novel approaches.
- Weak at: may skip rigor under time pressure; needs an explicit think-only
  instruction for planning.
- Register: smart, creative, expansive, moves fast.
- Cost class: not yet characterized for the standalone CLI in these sources;
  establish subscription or API billing from the configured access route.
- Typical order: after the default frontier pair for general work; may lead
  a frontend or creative procedure, subject to availability and billing.

**Antigravity** (Google)

- Strong at and weak at: not yet characterized. Account-dependent access to
  several model families does not establish one cognitive profile.
- Cost class: subscription/account-plan access or API-billed access, depending
  on the selected authentication method.
- Typical order: after the characterized frontier pair pending evidence for
  the selected model; API-billed access goes in the tail.

**Kiro**

- Strong at and weak at: not yet characterized. Custom agents and selectable
  models are capabilities, not comparative performance evidence.
- Cost class: account/plan access is documented, including higher-plan API-key
  authentication; whether that key is billed per call is not yet characterized.
- Typical order: after the characterized frontier pair pending performance and
  billing evidence; if API-billed, in the tail.

**Muse Code** (Meta)

- Strong at and weak at: not yet characterized.
- Cost class: API-billed with a separately billed API key; the cost class of
  account-login access is not yet characterized.
- Typical order: after the characterized frontier pair pending evidence;
  API-billed instances belong in the tail.

**fx**

- Strong at and weak at: not yet characterized. Several selectable backends
  mean a single model profile cannot be inferred from the CLI name.
- Cost class: not yet characterized; establish subscription or API billing
  from the configured backend.
- Typical order: after the characterized frontier pair pending evidence;
  API-billed backends belong in the tail.

**Cursor**

- Strong at and weak at: not yet characterized.
- Cost class: not yet characterized in these sources; establish subscription
  or API billing from the configured access route.
- Typical order: after the characterized frontier pair pending evidence;
  API-billed access belongs in the tail.

**Grok**

- Strong at and weak at: not yet characterized.
- Cost class: not yet characterized in these sources; establish subscription
  or API billing from the configured access route.
- Typical order: after the characterized frontier pair pending evidence;
  API-billed access belongs in the tail.

**OpenCode**

- Strong at and weak at: not yet characterized. Its model catalog and native
  planning agent do not establish comparative strengths.
- Cost class: usually API-billed; actual cost depends on configured access.
- Typical order: compare the configured route with suitable subscription-backed
  profiles; billing and capability determine placement.

**Pi**

- Strong at and weak at: not yet characterized. Provider models, extensions
  and skills do not establish a cognitive profile.
- Cost class: API-costing for this preference order. The CLI also supports
  provider login; that does not establish subscription coverage here.
- Typical order: compare the configured access route with other profiles;
  no personal fallback order defines instrukt defaults.

**DeepSeek Harness**

- Strong at and weak at: not yet characterized.
- Cost class: API-billed.
- Typical order: no special position in instrukt defaults; the user configures
  the order from the actual access route and capability.

**Other configured agent CLIs**

- This is an open extension category, not one CLI or model profile. Each
  additional configured CLI gets its own characterization as evidence grows.
- Strong at and weak at: not yet characterized.
- Cost class: not yet characterized; each agent brings its own authentication
  and models, so establish subscription or API billing per configured CLI.
- Typical order: after the characterized frontier pair pending evidence;
  billing and capability determine placement.

### Dispatch tiers

A tier selects the ordered model chain within each configured profile. Provider-native effort is a separate chain-entry setting, not the tier itself. Role mappings supply the tier; procedures declare only their role. Shipped chains use high effort throughout.

- **strong**: work whose error is costly and hard to see — novel design, analysis,
  planning, root-cause diagnosis, and every review except one of a
  documentation-only scope that directs no agent differently.
- **standard**: routine work against a settled plan — implementation, fixes,
  refactoring — and review of a documentation-only scope that directs no agent
  differently (`general/procedure/orchestration`).
- **light**: mechanical, clerical work whose result is checked at a glance.
- **premium**: an explicit higher-cost model chain, ending with the strong model as fallback. It is selected explicitly through configuration or a task override.

### Collaboration selection

When an agent needs a partner for collaborative discovery or sparring:

1. Select the complementary agent — the one whose cognitive profile covers your blind spots.
2. Resolve the collaborator's role mapping; **strong** is a shipped heuristic
   for substantial collaboration, and configuration owns the actual tier.

Default collaboration pairs:

- Claude partners with Codex: Claude brings architectural coherence, Codex brings thoroughness and alternative viewpoints.
- Codex partners with Claude: Codex brings meticulous analysis, Claude brings system-level reasoning.
- Gemini partners with Claude or Codex depending on domain: Claude for architectural decisions, Codex for implementation-heavy work.

The selection is not rigid. If the work is frontend-heavy, Gemini is the
right partner when it is available, whoever initiates. Read the domain, read the profiles, pick the complement.

## Why

Availability prevents an order from depending on a CLI that cannot start.
Performance comes next because a weak result consumes more work than a capable
agent saves in price. API billing adds marginal cost to each use, so API-costing
agents follow suitable subscription-backed choices. A strong parent maintains
coherence across delegated work; an ordered fallback keeps that work moving
when the preferred agent is unavailable.

The runner needs a reproducible selection rule. The agent composing the order
needs domain judgment, observed strengths and weaknesses, and the actual access
route. Keeping those responsibilities separate lets automation stay mechanical
while the preferences improve with evidence. Uncharacterized entries expose
what remains unknown rather than turning provider features into invented
performance claims.

Complementary profiles also improve collaboration: one agent catches what
another assumes. Thinking depth follows the cost of an invisible mistake;
reviews and directive documentation warrant deep attention, while mechanical
work needs less.
