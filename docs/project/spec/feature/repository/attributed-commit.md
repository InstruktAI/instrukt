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
| Active agent | the selected adapter's reported agent identity, when it can resolve one | the git user's own identity as author |
| Agent author | the global layer's `agents.<name>.git_author` for the active agent | refuse and name `agents.<name>.git_author` |

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

### Commit-review notes

`instrukt repo stamp <sha…> --verdict approve|needs_work --report <file>`
writes the verdict and the complete report's digest as a git note on each
named commit. The notes need no separate work record or enabled todos module.
The integrator reads these notes when assessing settled direct commits for
integration.

Each stamp is one line of a commit's note and carries its time,
`at: <RFC 3339 UTC>`, beside the verdict and its report digest or reason. A new
stamp is added beside the commit's earlier stamps, never replacing them, so a
merge never loses another machine's verdict. The latest stamp of a commit
decides its verdict. Stamps tied at the latest time settle the commit as
`approve` or `exempt` only when every one of them agrees; otherwise the commit
is unsettled.

The notes live under `refs/notes/instrukt-review`, and instrukt moves that ref
explicitly; no remote refspec is changed:

- **Push.** After writing the notes locally, `repo stamp` pushes the notes ref
  to the remote the default branch tracks, when there is one (the default
  branch is the remote's `HEAD`, then `init.defaultBranch`, then `main`).
- **Rejected push.** When another machine stamped first, the stamp fetches the
  remote notes ref, merges it into the local one (`git notes merge -s
  cat_sort_uniq`) and pushes again, a bounded number of times.
- **Unreachable remote.** The stamp still succeeds locally and reports the push
  as pending. Pushing the ref carries every local note, so the next stamp whose
  push succeeds delivers it; there is no queue and no flag.
- **Reading.** Every verdict reader — the integrator and the `repo` commands
  that read verdicts — fetches the notes ref first. When that fetch fails, it
  says so and reads the local notes.

`instrukt repo stamp <sha…> --verdict exempt --reason <text>` records
the author's own-stamp exemption and reason for a commit requiring no review.
The integrator takes commits whose latest stamp is `approve` or `exempt`,
stopping at the first commit that is not.

### Use cases

#### UC-RC1: An agent acting for a person commits its work under its own name

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

#### UC-RC2: A developer committing outside any agent session keeps their own identity

```gherkin
Given a developer at a terminal with no agent session in its environment
When the developer commits through instrukt
Then the commit is authored by the developer's own git identity
And the commit records no session
And the commit records whether a human was involved as the developer declared
```

#### UC-RC3: A developer makes settled direct work available for integration

```gherkin
Given a developer has direct commits ready to join the shared baseline
When an agent acting for the developer records their review outcomes or justified exemptions
Then settled commits become available for integration in order
And integration stops at the first commit without an accepting outcome
```

## Known caveats

- An agent with no configured git author is refused with the door `agents.<name>.git_author`.
- An entry with `agent: <name>` is an agent CLI.
- Without `--only`, commit includes what the caller staged, as plain git; `--only` commits exactly the given paths.

## See Also

- project/instrukt/spec/domain/repository
- project/instrukt/spec/domain/configuration
