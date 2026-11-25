---
description: An agent runs an onboarding from the state it finds to its outcome; where only a person can supply or do something, it prepares and guides that handoff, takes the work back and verifies the result.
---

# Onboarding — Policy

## Rules

- **An agent runs the onboarding.** The agent carries an onboarding from the
  state it finds to the stated outcome, through the tooling its role has.
- **The procedure names each handoff.** Where progress needs something only a
  person holds or can do — signing in, entering a credential through its
  protected channel, an approval, a decision that is theirs
  (`general/policy/human-contract`), a fact about their own world — the
  procedure names the person, what that person contributes, what the agent
  prepares for it, and the result that returns the work to the agent. Every
  other step is the agent's. An onboarding procedure requires this policy
  under its Required reads.
- **The agent guides the handoff and takes the work back.** It gives the
  person the next action they can take now, where to take it and what the
  agent has prepared, and says what tells them their part is done. When the
  agent can observe the result, or the person confirms what only they can
  observe, the agent resumes the work and verifies the stated outcome.
- **The agent accounts for missing automation.** It uses the supported tools
  available to its role. When required automation is missing, it reports what
  remains unfinished through the procedure's reporting route and continues the
  work that can proceed.

## Rationale

- The person being onboarded knows their own world and holds their own
  credentials; the platform is what the onboarding spares them. With the
  handoff prepared and its end named, the person completes their part without
  having to coordinate the remaining setup.
- A step that names no owner leaves the person to work out whether it is
  theirs.
- A handoff with no return leaves the outcome unverified: the person's part is
  finished and nothing has established that the setup works.
- Work done by hand where automation was required hides that the automation is
  missing: this onboarding then differs from the next, and every later run
  meets the same gap. Reporting it as unfinished keeps the outcome honest.

## Scope

Every agent that writes, reviews or runs an onboarding procedure.

## Enforcement

Reviewers apply this policy to onboarding procedures. When a person reports
being left to coordinate setup, the handoff is reviewed in the run and in the
procedure.

## Exceptions

None.

## See Also

- [general/policy/leadership](leadership.md)
