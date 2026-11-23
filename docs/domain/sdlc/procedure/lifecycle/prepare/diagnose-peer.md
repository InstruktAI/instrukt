---
eval: true
description: Dispatched bug-diagnosis peer procedure. Investigate independently, establish the causal mechanism, and author the diagnosis artifact without taking over prepare orchestration.
visibility: internal
role: diagnoser
parameters:
  - name: slug
    required: true
---

# Peer Bug Diagnosis — Procedure

## Required reads

- [general/policy/human-involvement-recording](../../../../../general/policy/human-involvement-recording.md)
- [general/concept/failure-modes](../../../../../general/concept/failure-modes.md)

## Goal

Independently investigate a reported bug and write the `diagnosis` artifact that the
prepare-driving session will interrogate. The peer establishes why the observed behavior
occurs and stops at the causal mechanism; it does not triage the todo landscape, dispatch
another session, prescribe a fix, stamp the diagnosis, or drive preparation.

## Preconditions

- The prepare-driving session dispatched this session for a todo whose kind is `bug` and
  whose current derived obligation is diagnosis.
- `instrukt todo artifacts read {slug} input` contains the reported symptom and discovery
  context.
- This session is recorded as the diagnosis peer for the todo. The
  prepare-driving session retains the execution claim; the peer owns
  investigation and revisions of `diagnosis`.

## Steps


1. Read `instrukt todo show {slug}` and the `input` artifact. Use the creation record to locate
   the organic trigger and its evidence surface. `human_involved` records participation, not
   authorship or ratification of every detail. Treat agent-added causes, remedies, numeric
   bounds, and adjacent scope as hypotheses.
2. Treat the symptom as a real defect until evidence proves otherwise. Reproduce where
   possible, run the bounded provenance check below, bisect the failing path, and isolate
   one falsifiable causal mechanism. Label synthetic reproduction and state any gap from
   the organic path.
   - **Provenance check (bounded lane — unconditional for every bug):** determine _since
     when this is reachable_. Identify the files on the failing path and anchor on the
     mechanism (`git log --oneline -S<symbol> -- <files>` or `-G<pattern>`). If the anchor
     does not settle it, run at most one range scan (`git log --oneline -- <files>`) over
     the plausible window and inspect candidate commit diffs. Supplied commits are
     candidates, not the boundary of the search. Record exactly one verdict:
     - **Regression** — name the commit and quote the diff hunk that broke working behavior.
     - **Latent** — name both the introducing commit and the change that made it reachable,
       with diff hunks.
     - **Provenance not established** — record the files, commit range, pathspecs, and
       `-S`/`-G` terms searched and move on. Reaching this exit after the anchor and one
       range scan completes the lane; do not keep widening the search.
     - What a named commit touched beyond the mechanism is evidence for separate triage
       (`sdlc/policy/todo-creation-discipline`), never scope for this bug.
3. Apply the **operation-outcome and failure-handling distinction**: establish the
   operation's expected outcome under the observed conditions from its applicable contract,
   and separate a failure to meet that outcome from a defect in reporting or handling it.
   Handling that controls execution, recovery, or resulting state participates in the
   causal chain; a presentation-only cause is sufficient only when evidence establishes
   that the underlying outcome conforms to contract, and a shared error signature does not
   establish a shared cause.
   Record the operation, the source and applicability of its expected outcome, and the
   evidence supporting the investigation boundary. Trace backward to the earliest violated
   contract inside that boundary. Record the complete minimal causal set, including
   enabling conditions such as races, ordering, or omitted handling. When the root is a
   shared contract, enumerate every producer, entry path, and consumer in the boundary and
   classify each from observed evidence.
   Account for the reported occurrences individually or in evidence-supported causal
   groups. State which transitions a reproduction supports and which remain unsupported
   for the reported occurrences. Record unexplained occurrences and the missing evidence
   without representing them as explained by the reproduction.
4. Write `diagnosis` with `instrukt todo artifacts write {slug} diagnosis <content>`. Keep
   observations separate from the argument that cites them, and include:
   - **Status** — `Draft`, `Settled`, or `Unresolved`.
   - **Symptom** — expected versus observed behavior, the expected outcome's source and its
     applicability to the observed conditions, organic trigger, reproduction status, and
     any synthetic-to-organic gap.
   - **Provenance** — exactly `Regression`, `Latent`, or `Provenance not established`, with
     the evidence required by step 2.
   - **Observations** — what was seen, how it was obtained, and why it was inspected.
   - **Causal trace** — every transition, its carrying observations, its falsifier, comparable
     near misses, occurrence coverage, and the implementation census when the root is shared.
   - **Alternatives** — the strongest credible rival and the discriminating check actually
     run, or the reason no credible rival survives statement.
   - **Root cause** — the minimal causal set and its counterfactual, with no fix.
   - **Mechanism** — state the root cause twice: the failing instance and the code shape
     that produced it, narrow enough that a reader can confirm or reject it at one site by
     reading that site. When a governing design or decision record prescribes it, name that
     contract by id, and state whether its design's `Failure modes` already record this
     failure.
   - **Family census (bounded lane)** — at most two searches: the diff of the introducing
     commit (when provenance named one) and one search of the codebase for the mechanism's
     shape. Record the searches run and every site found, each marked `matching`,
     `not matching`, or `undecided`. Do not investigate a family site's reachability,
     likelihood, or impact: a match is a lead, not evidence of a defect there
     (`sdlc/concept/defect-evidence`, family site).
   - **Attack record** — initially empty or marked pending; the peer updates it when the
     prepare-driving session later challenges the written mechanism.
5. Write the `diagnosis` artifact, then go idle for the prepare-driving session's response. If the prepare-driving session returns an evidence gap or
   challenge, gather the missing evidence, revise the artifact yourself, and update the
   Attack record with the challenged transition, evidence, and resulting movement. The peer
   keeps the pen for every diagnosis version.

## Outputs

- A `diagnosis` artifact whose observations are independently auditable and whose causal
  trace either establishes the root cause or states precisely what remains unresolved.
- No production-code, test, worktree, commit, fix, verification prescription, lifecycle
  stamp, todo transition, or child session.

## Discipline

You are the dispatched diagnosis peer, not the prepare-driving session. Do not triage,
dispatch another worker, interrogate your own artifact, stamp the diagnosis, or advance the
prepare state machine. Investigate independently, write `diagnosis`, prescribe no fix, and
wait for the driver to challenge or accept the evidence.

This session is the dispatched diagnosis peer, never the prepare driver. Do not perform
triage, dispatch another diagnoser, interrogate your own artifact, stamp a verdict, or call
`instrukt todo prepare`; those acts belong to the driving session. Follow evidence backward to
the earliest violated contract, write only what the observations carry, and stop before a
fix or verification approach.

## Recovery

- If no cause grounds, write `Status: Unresolved` with the verified observations, competing
  mechanisms, and the observation that would separate them; do not invent a likely cause.
- If the behavior is correct, record that conclusion and its evidence in `diagnosis`; the
  driving session owns reclassification or closure.
- If the cause is a process-design gap, diagnose and report it without changing the process;
  the driving session owns the authority escalation.
- If an artifact write is refused, preserve the refusal and report it to the
  driving session. A remote revision conflict follows `instrukt todo refresh
  {slug}` and reapplication of the intended change; no alternate write path
  or transfer of the driver's claim is implied.
