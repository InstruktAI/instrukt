---
description: 'Prose is not code: a reader judges it against intent and it is never tested. Code that grades free text substitutes automation for judgment, and the impulse to build a validator is the warning sign.'
visibility: 'public'
---

# Intelligence Boundaries — Principle

## Principle

When the question is "is this content meaningful, important, complete, on-topic, or correct?", the answer comes from intelligence — the agent reading it — not from code that grades it. Code that classifies, scores, judges, evaluates, or auto-edits free-text content substitutes automation for judgment, and the substitution decays.

Prose is not code. Documentation, procedures, policies, prompts and skills are instructions agents are trusted to follow, and a reader judges them against the intent they are meant to convey. Prose itself is never tested: no test, replay or score of it establishes that it is right, whether code runs the check or agent sessions are staged to. A software check establishes software behavior, such as how a parser reads a document or how a renderer delivers it. `general/policy/documentation-discipline` governs review of the text, and evals compare outcomes of recorded work over time (`sdlc/procedure/eval-report`).

The same boundary keeps agent coordination out of code and product feature specifications. Tools record neutral facts and leave workflow choreography, role handoffs, and turn order to briefs and procedures (`general/principle/trust-and-guardrails`); product feature specifications contract software state and observable tool behavior, leaving agent-to-agent coordination to procedures.

## Rationale

Heuristics built on regex, word counts, or hardcoded vocabulary are correct only against today's language. They go wrong silently when anything shifts. Auto-editors are worse: they rewrite the agent's intent to satisfy their own grading. Once a rubric exists in code, agents learn to write to the rubric — the rubric becomes the artifact and the artifact becomes hollow.

A replay records how an agent acted on one occasion. The standard for what the instruction was meant to convey comes from its authorized intent.

## Implications

- When the impulse arises to _build a validator / classifier / scorer / checker for X_, X is almost always semantic. The impulse is the warning sign. Load `sdlc/policy/no-semantic-validators` before writing the check.
- Policy on agent output and agent-to-agent choreography lives in the agent's briefing and procedures, not in transport, validation, CLI policing, or scoring code that wraps the agent.

## Tensions

- **Defense-in-depth vs. trust.** Layered checks feel safer; each layer compounds drift when the heuristic shifts. Fewer layers, better briefings.
- **Automation cost vs. agent cost.** Code feels cheaper than agent inference. Over the lifetime of the codebase, code that grades content is more expensive — drift, gaming, removal — than the inference it replaced.
