---
description: 'Resolving conflicts between docs, instructions, and code.'
visibility: 'public'
---

# Conflict Resolution Rules — Policy

## Rules

- Resolve conflicts between governing contracts by priority: specificity → rule
  intent → applicability → recency.
- Work artifacts (such as task inputs, requirements, architectures, and plans)
  authorize delivery scope; they stand outside this precedence ladder and grant
  no exemption from standing principles, policies, or verified substrate reality
  (`general/policy/human-contract`).
- Code is ground truth for current behavior; documentation is the contract for
  intended behavior.

## Rationale

- Consistent priority prevents indecision and hidden rule‑picking.
- Delivery inputs are authored before full grounding; separating them from
  governing contracts preserves standing invariants across every review gate.
- Separating observed behavior from documented contract keeps code bugs from
  silently rewriting specifications while preventing stale docs from masking
  what the system actually runs.

## Scope

- Applies to all agent instructions, project docs, and tool guidance.

## Enforcement

- If two rules conflict, choose the higher‑priority rule and proceed.
- Note the conflict in the final report if it affects outcomes.

## Exceptions

- None.
