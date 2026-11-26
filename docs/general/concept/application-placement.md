---
description: 'How an application a company uses is prepared and placed: its knowledge states preparation as requirements an agent checks, the hosting of one deployment is the knowledge of the project that runs it, and each company records where its applications run — on which platform, shared or dedicated, run by which project.'
---

# Application Placement — Concept

## What

**Operating an application assumes a prepared one.** Each application's
knowledge states its preparation as requirements an agent can check: the state
a ready application is in, not the steps that bring it there. Where the
application already runs as a hosted service, the agent checks the
requirements off. On a platform no knowledge covers, the agent meets them by
its own judgment; this concept supplies the requirements, not the steps.

**Operating knowledge and hosting knowledge have different owners.** Knowledge
of operating an application — how a company's work is done in it — is domain
knowledge, reusable across every company that uses the application. Knowledge
of preparing and hosting one particular deployment of it is project knowledge,
owned by the project that runs that deployment.

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
- A deployment's hosting is true for one project's machines. Kept in domain
  knowledge it would be wrong for every other company using the application,
  and the domain would stop being reusable.
- Where an application runs decides which machine an agent acts on and which
  platform's knowledge applies. Inferred from appearances, it sends the agent
  to the wrong place; recorded in the company's knowledge, it has one owner
  and can be checked.
- A shared start lets a company begin without a machine of its own; isolation
  is a need a company grows into, so the move is a named transition whose
  mechanics live with the platform that performs it.
