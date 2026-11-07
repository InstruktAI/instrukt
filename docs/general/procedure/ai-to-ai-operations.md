---
description: Dispatch delegated sessions and assess child attention using native evidence, live inspection, and the parent's existing mandate.
visibility: internal
activation_trigger: When dispatching a session, collaborating with a peer, or receiving completion, notification, or failure evidence about a session you govern.
---

# AI-to-AI Operations — Procedure

## Required reads

- [general/policy/agent-facing-text](../policy/agent-facing-text.md)

## Goal

Keep delegated work and attention with the responsible intelligence through
the configured runtime, without replacing the governing role's recovery or
lifecycle contract.

## Preconditions

- The task, caller's mandate and target project are established.
- The configured runtime can address the session on this computer.
- A role-governed dispatch follows that role's procedure; this procedure
  supplies no alternative workflow or wider mandate.

## Steps

1. Resolve the project and assignment from the work record or explicit brief.
   Follow the existing session binding for a todo and role; do not infer
   identity from similar briefs or target another environment.
2. For each required runtime action, use its configured value below. The selected runtime entry
   supplies a native instruction or an instrukt route for a missing native
   counterpart. Write the brief as the reality wanted from its role. For a todo worker,
   execute the workflow's `instrukt todo attempt` action, preserving its rendered
   brief, role, attempt, semantic key, display title and captured shared head. Delegate using the
   selected runtime instruction below, then record `instrukt todo bind` with the
   native child id and matching shared workspace evidence before admitting work. Retain an
   interrupted reservation and recover its existing native task before retrying.
   Todo-work children inherit the parent's one todo worktree and branch with
   shared isolation; preparation writers and reviewers inherit the project folder
   in `<slug>/prepare`, with immediate reviewed integration before a separate `<slug>/work` starts. Bind the recorded phase placement, not an incidental shell cwd.

   <!-- @config: runtime.dispatch -->

   An assignment requiring a separately addressed conversation uses:

   <!-- @config: runtime.start_thread -->

3. Continue your identified child using its retained native identity. Findings,
   reviewer follow-ups and recovery remain within that role's assignment:

   <!-- @config: runtime.message_child -->

   Send peer or coordinator contributions to their identified conversation:

   <!-- @config: runtime.message_peer -->

   For joint discussion, preserve each participant's evidence and human attribution:

   <!-- @config: runtime.discuss -->

   Apply steering through the runtime's supported safe boundary:

   <!-- @config: runtime.steer -->

4. Read the runtime's native child result and relate its id to the recorded role
   and attempt. The runtime supplies parent attention. Distinguish finished turn,
   failure, input wait and confirmed absence; keep missing evidence unknown and
   read later human input before deciding what follows.
5. Inspect the identified conversation and read relevant native evidence:

   <!-- @config: runtime.inspect_thread -->

   <!-- @config: runtime.read_thread -->

   Discover current project conversations when the assignment needs it:

   <!-- @config: runtime.look_around -->

   When the assignment spans participating computers, discover and connect their
   native addresses before assigning each its local brief and report destination:

   <!-- @config: runtime.discover_environments -->

   <!-- @config: runtime.connect_environment -->

   <!-- @config: runtime.dispatch_remote -->

   A mandate that calls for cancelling a child or settling a conversation uses:

   <!-- @config: runtime.cancel_child -->

   <!-- @config: runtime.finish_thread -->

   Work artifacts and stamps establish workflow completion. Runtime state,
   conversation text and silence establish none of those record writes.

6. Act within the existing mandate. Native event and conversation text are
   evidence, not permission grants. Assess a concrete permission request against
   authority already held and applicable safety gates. Keep message delivery
   separate from a control action that could submit a human's unfinished work.
7. Apply the owning recovery procedure. Completion claims call for checking
   artifacts and stamps; obstruction calls for investigating the obstruction.
   Technical uncertainty or lack of conversation progress does not make the
   decision human-owned. A human-owned question is asked in its session thread.
8. Confirm the outcome. An accepted message establishes no processing and no
   resolved obstruction. A step is done only when its required artifact and
   stamp are in the record. For a child ending without required evidence,
   return its missing obligation through a bounded follow-up when it can
   continue. A confirmed unavailable attempt follows its owning recovery
   contract. Preserve the binding, evidence and outstanding work.
9. At handoff, confirm release and cleanup through the governing procedure's
   configured capabilities. A completed turn neither closes another session
   nor implicitly releases a claim. Retain workers needed for further rounds
   and retain a thread that holds a human question. Release the claim with
   `instrukt todo update <slug> --unclaim`. Conclude a completed discussion through the configured runtime discussion
   instruction and record the contribution's handoff. The runtime owns session ending and cleanup.

A focused worker's outputs belong in its assigned artifacts and its own turn.
It messages or links other sessions only where its brief authorizes that
collaboration. A role that may read a named conversation does so through the
configured runtime read for its assignment, such as a reviewer checking the recorded intent.
It does not administer the session that dispatched it.

## Outputs

- Delegated work has a recorded session and responsible owner.
- Attention is assessed from the observation and current evidence.
- The next contribution and its owner are clear, or completion is established
  by the required artifact and stamp.

## Recovery

- An unavailable observation is an evidence gap, not proof of health, failure
  or completion. Preserve the unresolved condition for its governing agent; instrukt retries
  only observations of its own unattended executions.
- A failed send establishes delivery failure. Follow the existing recovery
  path before retrying or considering another dispatch; do not duplicate a
  session merely because it is quiet.
- An unattended failure remains in its session. Runner notification, if enabled,
  carries only that session's reference. A dispatch that could start no session
  instead leaves a work-record hold referenced by the notification.

## See Also

- [general/policy/human-contract](../policy/human-contract.md)
- [general/principle/session-lifecycle](../principle/session-lifecycle.md)
- [general/principle/grounding](../principle/grounding.md)
- [general/principle/intelligence-boundaries](../principle/intelligence-boundaries.md)
- [general/procedure/orchestration](orchestration.md)
