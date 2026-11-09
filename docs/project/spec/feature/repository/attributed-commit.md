---
description: Attributed commits — `instrukt repo commit` authors an agent's commit as that agent, records its session and the caller's declaration of human involvement, names the work item when given, refuses caller-written trailers, and outside any session leaves the developer's own identity untouched.
delivered_by: [instrukt-first-build]
---
# Attributed Commit — Spec

## What it is

A commit records who did the work and under which circumstances. When an
agent commits through `instrukt repo commit`, the commit is authored by that
agent, names the session it ran in, carries the caller's declaration of
whether a human was involved, and names the work item when one is given.
These facts come from the caller's environment and configuration, never from
guesswork: what cannot be resolved is left out, and outside any agent session
the commit is the developer's own. The trailers are instrukt's alone; a
message that writes them itself is refused.

## Canonical fields

### Resolution

| Fact | Source | Unresolved |
| --- | --- | --- |
| Active runtime and its session id | the explicitly selected adapter's recognition of the caller's native session (`project/instrukt/spec/domain/configuration`) | no `Session:` trailer |
| Bot-role author | the recognized session's recorded role and configured App's verified bot login/id | refuse with the App identity configuration door |
| Active agent outside a bot role | the selected adapter's reported agent identity, when it can resolve one | the git user's own identity as author |
| Non-bot agent author | the global layer's `agents.<name>.git_author` for the active agent | refuse and name `agents.<name>.git_author` |

A recorded bot role uses its selected App's bot noreply author under
`project/instrukt/spec/feature/work/github-apps`, before provider-agent lookup.
The bot account id, not the App id, forms the noreply address. Local author
resolution consumes the machine's complete setup-verified bot identity through
work's actor capability. It needs no GitHub-hosted repository, App private key,
installation token or remote write authorization for the local repository. Unmapped agent
roles use the project App. This works with T3's recognized native session even
when T3 reports no provider-agent identity. A known role with unavailable App
identity refuses rather than writing under the developer's name. A roleless
human session keeps the existing non-bot resolution.

The runtime choice is explicit; recognising caller identity does not select one.
An adapter may identify the provider of its recognised session, but an unknown
agent identity stays unresolved rather than being inferred from an ordered
configuration entry. The adapter's metadata carries the observation.

### The message

- The caller supplies one message: a subject and an explanatory body.
- instrukt appends, after a blank line, in this order:
  - `Session: <runtime>:<id>` — when a runtime resolves;
  - `Human-Involved: true|false` — always; the caller's declaration through
    `--human-involved true|false`, `false` when omitted;
  - `Todo: <slug>` for a recorded work item, or `Planned-Todo: <slug>` for a
    planned association — when the caller supplies `--todo`.
- **Refusal.** A message containing a line whose key is `Session`,
  `Human-Involved`, `Todo` or `Planned-Todo`, in any letter case, is refused before anything
  is committed; the refusal names the options that supply those facts.
- The work-item slug is checked for form; whether the item exists is checked
  only when the todos module is on (`project/instrukt/spec/domain/work`).
  With todos off, `--todo <slug>` writes `Todo: <slug>` as given; it never
  infers a planned association. With todos on, recorded and planned
  associations follow the existence check above.

### What is committed

- `--only <paths>` commits exactly those paths as they are in the working
  tree, new files included, and leaves everything else in the index as it
  was.
- The project's own git hooks run as for any commit; a hook that refuses the
  commit leaves nothing committed.
- Works in any git repository; the project layer is not required.

### Worktree commits and review

Only the integrator writes main. `repo commit` refuses the canonical branch and directs authoring to a todo or short-lived ad hoc worktree. Review binds the committed worktree head before merge; the complete report and verdict accompany that candidate. The per-SHA direct-main `repo stamp` surface and review notes are removed.

## Use cases

### UC-RC1: An agent acting for a person commits its work under its own name

```gherkin
Given an agent working for a developer in a session of a configured runtime
And the agent's git author is configured on the computer
When the agent commits its work, declaring that the developer was involved, for a named work item
Then the commit is authored by the agent
And the commit records the runtime and session it was made in
And the commit records that a human was involved and names the work item
When the agent's message itself contains a session line
Then nothing is committed and the agent is told how to supply that fact
```

### UC-RC2: A developer committing outside any agent session keeps their own identity

```gherkin
Given a developer at a terminal with no agent session in its environment
When the developer commits through instrukt
Then the commit is authored by the developer's own git identity
And the commit records no session
And the commit records whether a human was involved as the developer declared
```

### UC-RC3: Direct-main review notes (retired)

Retired: no agent authors direct commits on main. Worktree-head review precedes integration.

## Known caveats

- T3's caller lookup supplies a native session without verified provider-agent
  identity. A bound bot role resolves through the recorded role and verified App;
  the unresolved-agent fallback applies only outside that path. Selection order
  never supplies identity evidence.
- Outside the bot-role path, a resolved agent with no configured git author is
  refused with the door `agents.<name>.git_author`.
- Without `--only`, commit includes what the caller staged, as plain git; `--only` commits exactly the given paths.

## See Also

- [project/instrukt/spec/domain/repository](../../domain/repository.md)
- [project/instrukt/spec/domain/configuration](../../domain/configuration.md)
