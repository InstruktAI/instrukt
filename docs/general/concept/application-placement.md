---
description: 'How a company''s applications are prepared and placed: preparation as requirements an agent checks, and a recorded placement — platform, shared or dedicated, the project that runs it.'
---

# Application Placement — Concept

## What

**Operating an application assumes a prepared one.** Each application's
knowledge states its preparation as requirements an agent can check: the state
a ready application is in, not the steps that bring it there. Where the
application already runs as a hosted service, the agent checks the
requirements off. On a platform no knowledge covers, the agent meets them by
its own judgment: the application's knowledge supplies the requirements,
and no knowledge prescribes the steps.

**Operating knowledge and hosting knowledge have different owners.** Operating
an application is knowledge of its domain, and the preparation and hosting of
one deployment of it is project knowledge (`general/policy/artifact-scope`),
held by the project that runs that deployment as the owner of that deployment
fact (`general/policy/snippet-value-contract`).

**A company's placement is a recorded fact.** The company's knowledge records,
for each application it uses, the platform it runs on, whether it is
**shared** (several companies on one machine) or **dedicated** (the company's
own machine), and the project that runs it. An agent takes the placement from
that record; a placement the record does not state is unknown. How things are
done on a given platform is that platform's own knowledge, loaded because the
company's record names the platform.

**A company starts shared and moves to dedicated by a defined transition.**
Shared placement is where a company starts, as in a trial or onboarding.
Moving to dedicated is taken when the company needs isolation, and each
platform's knowledge says how the move is done there. Commercial terms —
pricing, who pays — are each operator's own and outside this concept.

## Why

- A requirement stated as state holds on every platform and lets an agent
  confirm a ready application without repeating its setup; steps hold only on
  the platform they were written for.
- Where an application runs decides which machine an agent acts on and which
  platform's knowledge applies. Inferred from appearances, it sends the agent
  to the wrong place; recorded in the company's knowledge, it has one owner
  and can be checked.
- A shared start lets a company begin without a machine of its own. A move to
  its own machine relocates everything the company holds in the application,
  so it follows a known path instead of one improvised for the occasion.
