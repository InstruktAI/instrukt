---
description: 'Guidelines for resolving conflicts between documentation, instructions, and code.'
visibility: 'public'
---

# Conflict Resolution Rules — Policy

## Rules

- Resolve conflicts between governing contracts of the same standing by
  priority: specificity → rule intent → applicability → recency.
- Work artifacts (such as task inputs, requirements, architectures, and plans)
  authorize delivery scope; they stand outside this precedence ladder and grant
  no exemption from standing principles, policies, or verified substrate reality
  (`general/policy/human-contract`).
- Treat documentation as direction; use code to confirm behavior.

## Rationale

- Consistent priority prevents indecision and hidden rule‑picking.
- Keeping work artifacts off the contract-precedence ladder ensures an
  unverified claim or mechanism inside a task input never overrides standing
  engineering invariants.
- Code is the ground truth when docs and behavior diverge.

## Scope

- Applies to all agent instructions, project docs, and tool guidance.

## Enforcement

- If two rules conflict, choose the higher‑priority rule and proceed.
- Note the conflict in the final report if it affects outcomes.

## Exceptions

- None.
