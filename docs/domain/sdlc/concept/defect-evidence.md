---
description: 'The terms that ground a claimed defect — occurrence, dangerous path, reachable condition, tail risk, and external-substrate boundary — shared by filing, triage, building, and review.'
visibility: 'public'
---

# Defect Evidence — Concept

## What

These terms carry the evidence for a claimed defect. Every contract that judges
a defect claim — filing it, triaging it, building around it, or raising it in
review — uses them.

- **Occurrence** — the defect happened in real use. A person or system acting
  for its own purpose produced the triggering input and state, and an address a
  reader can open shows it: a log record, a delivered message, a session
  transcript, a person's account of what they experienced. Diagnosis calls this
  the organic trigger. Input or state that an agent's test, probe, smoke run, or
  reproduction produced is synthetic, whatever that agent intended; it can
  reproduce an occurrence but is never one. A required CI check that fails on
  main or on a dependency update is an occurrence of that revision failing the
  check: the pipeline runs for its own purpose of admitting revisions, and the
  run is the address. It shows that the check failed on that revision, not
  which change or component is at fault.
- **Dangerous path** — a concrete path to a defect that has not occurred. It
  names who acts, through which surface, holding what access, doing what, and it
  ends in a consequence that meets the safety gate's bar in
  `general/policy/human-contract`: material, hard-to-reverse exposure of a
  human-owned asset. Hard-to-reverse is judged by what is lost, leaked, or done
  in someone's name before anyone can intervene. A transient, self-clearing, or
  cosmetic consequence does not meet the bar, and neither does a state that
  only waits, losing nothing, until someone notices and undoes it. The path
  takes one of two forms:
  - **Ordinary use** — every step is an act the actor performs through the
    surface's ordinary operation, with real-use inputs. Conditions reached only
    by staging artificial failure sequences, tampering with persisted state,
    sabotaging runtime permissions or environment, or injecting timing controls
    between operations are synthetic fault injection, never ordinary use — even
    when executed through real external dependencies.
  - **Deliberate misuse** — an actor whose relationships grant it no reach to
    the exposed asset constructs inputs that reach or impair that asset. This is
    the only form in which constructed inputs count. An agent acting on such an
    actor's input is the surface of the path, not its actor. Agents, and any
    party acting within the reach it was granted, are otherwise trusted
    (`general/principle/trust-and-guardrails`) and are never this actor.

  "Someone could" names no actor and is not a path.

- **Family site** — a site a settled diagnosis's family census marks as matching
  its mechanism (`sdlc/procedure/lifecycle/prepare/diagnose-peer`). It is a recorded lead and carries
  no evidence: the diagnosis's occurrence or dangerous path stays with the site
  where it happened. A family site becomes a defect claim only on evidence of
  its own — an occurrence at that site or a dangerous path through it — against
  a current contract that states the expected behavior.

- **Real-use input** — an input real use produces, shown rather than asserted:
  an observed instance of it in real use, even one that triggered no defect, or
  a producer the code actually receives input from that emits it. The
  theoretical range of an input's type is not evidence that real use produces
  any particular value in it.
- **Reachable condition** — a condition that has occurred, that real-use input
  produces, or that a dangerous path reaches. Any other condition is
  unreachable, including one real use might produce but no shown real-use input
  or dangerous path reaches. A condition resting on synthetic fault injection,
  extreme tail risk, or missing guarantees across an external-substrate
  boundary is not a reachable defect condition. Having been produced so far
  only by tests decides nothing either way; for code that has not shipped, a
  producer it actually receives input from shows its real-use input.
- **Tail risk** — a theoretically constructible interleaving or compounded
  chain of independent rare faults whose real-world likelihood sits in the
  extreme tail, especially where no irrecoverable human-owned asset is lost.
  Intelligence weighs real-world likelihood against actual harm without
  formulas or arbitrary thresholds; extreme-tail conditions fall outside
  engineering scope and are not defects to build defensive machinery around.
- **External-substrate boundary** — the boundary between the system's own
  contract and capabilities an external dependency does not supply. The system
  enforces its own contracts and handles declared failure surfaces; it does not
  fabricate transactional atomicity or guarantees across external boundaries
  that do not provide them, and missing atomicity in an external dependency is
  not a defect in the caller.

## Why

A defect that only synthetic input, fault injection, extreme-tail interleavings,
or missing guarantees across an external-substrate boundary produces changes
nothing anyone experiences in ordinary use. Reporting it spends a diagnosis or
review cycle, and handling it adds brittle defensive machinery — often
oscillating against mutually exclusive synthetic cases — without improving
real-world safety. Each term ties a claim to something a reader can check — an
address, an observed input, a named actor, an access, an act, and the boundary
of what the system owns — rather than to how plausible the claim sounds, so two
agents reading the same claim classify it the same way. A report can always
assert a malicious actor or a user who "pastes anything"; only shown evidence
names one.
