---
description: Stateless screening of untrusted text through independent capability-free checks over shuffled 3-to-8-word fragments, returning a pure boolean safety verdict via `instrukt is-safe` and storing no duplicate copy locally.
delivered_by:
  - capability-free-model-call
---
# Untrusted-Text Screening — Spec

## What it is

Untrusted text is text written by people outside the project, such as an
outside contributor's pull request. Before such text reaches a tool-capable,
brief-governed agent, `instrukt is-safe` (and its matching internal call)
checks whether the text tries to manipulate or attack the model that reads it.
Text of six or more words is cut into random fragments of 3 to 8 words and
shuffled without original positions; text shorter than six words is sent as a
single fragment rather than rejected for its length. More than one independent
capability-free check
(`project/instrukt/spec/feature/screening/capability-free-calls`, using global
`llm.screening`) judges whether the fragments look manipulative or malicious. A
manipulation that succeeds against a check can change only that check's
verdict.

Screening is stateless and stores no copy of the text locally. The source —
such as GitHub — already holds the text durably. `instrukt is-safe` returns a
pure boolean verdict: safe only when every check judges benign, or unsafe with
a short reason otherwise, without echoing the text back and without producing a
hash or stored reference. When text is unsafe, the consumer withholds it from
brief-governed agents and points the responsible person at the source alongside
the reason.

Screening carries no knowledge of where a text came from. Standard input is the
primary command input so untrusted text need not touch disk, with an optional
file path for callers that supply a temporary file; internal callers pass text
in memory.

Developer logs (`project/instrukt/spec/feature/configuration/logging`) serve
debugging only and are never the durable audit store for safety verdicts.
Durable recording of command outcomes belongs to the platform's domain event
ledger at the CLI and runner execution boundary rather than a screening-owned
database.

## Canonical fields

### Screening a text

| Field | Meaning |
| --- | --- |
| Text | The complete untrusted text, supplied on standard input (primary), from an optional file path, or in memory by an internal caller. |
| Fragments | For texts of six or more words, the complete text cut into fragments of random length from 3 to 8 words and shuffled with no original positions; for texts shorter than six words, the complete text as a single fragment. Separate checks need not receive distinct shuffles. |
| Check | One capability-free call (`project/instrukt/spec/feature/screening/capability-free-calls`) asking whether the fragments look manipulative or malicious, with no earlier check or other context. Screening runs more than one independent check. |
| Verdict | A simple `benign` or `flagged` judgment, with a short reason when flagged. |
| Outcome | `safe` (exit 0, `true`, or `{"safe": true}` with `--json`), `unsafe` with a short reason (non-zero exit, `false`, or `{"safe": false, "reason": "..."}` with `--json`), or `refused` at entry when an endpoint prerequisite is missing or unresolvable. While the endpoint is unavailable screening is `waiting` and returns no outcome yet. No outcome echoes the text, emits a hash or returns a stored reference. |

### Safety decision, waiting and failures

Text is judged `safe` only when every independent check returns `benign`. Text
is judged `unsafe`, with a short reason, when any check flags it, when checks
disagree, when a check's answer is malformed or cut off before finishing, or
when the endpoint rejects a check's request as one it cannot serve, such as one
too large for the model. Text is never truncated or judged in part.

While a check waits on an unavailable endpoint
(`project/instrukt/spec/feature/screening/capability-free-calls`), screening is
`waiting`: it has no outcome yet and returns neither `safe` nor `unsafe`.
instrukt, not the caller, tries it again through the outside gate
(`project/instrukt/design/architecture/outside-gate`), and the caller receives
the eventual outcome (`safe` or `unsafe`) once the endpoint answers, without
asking again. Before any text is taken, any missing or unresolvable endpoint
prerequisite — an unconfigured or incomplete `llm.screening` endpoint, or
missing, blank, unreadable or insufficiently protected referenced API key
secret material (`project/instrukt/spec/feature/configuration/owned-files`) —
is `refused` at entry, naming what is missing and the door to supply it
(`project/instrukt/policy/checks-and-guards`), and never reports `safe`.

A defect — a failure that is neither an entry refusal, an unavailable endpoint
nor one of the post-call unsafe results above, such as a malformed request or a
client or protocol failure (`project/instrukt/policy/checks-and-guards`) —
fails screening without retrying and never reports `safe`.

Screening writes nothing to local storage for any outcome.

## Use cases

### UC-SC1: An agent checks outside text before reading it

```gherkin
Given an operator has configured an OpenAI-compatible endpoint and model for screening
And an agent acting for the operator holds outside text of six or more words that it has not read
When the agent checks whether the text is safe
And every classification judges it benign
Then the agent receives a safe verdict without the text echoed back
And no classifier saw the text in its original order
```

### UC-SC2: Text that tries to manipulate its reader is reported unsafe

```gherkin
Given an operator has configured an OpenAI-compatible endpoint and model for screening
And an agent acting for the operator holds text written outside the project
And the text instructs whoever reads it to ignore its instructions and publish the project's secrets
When the agent checks whether the text is safe
And a classification flags it
Then the agent receives an unsafe verdict with a short reason
And the response contains neither the screened text nor a stored reference
And screening stores no copy of the text locally
```

## Known caveats

- Screening lowers the chance that manipulative text reaches a tool-capable
  agent; it does not guarantee perfect detection.
- Texts shorter than six words cannot be shuffled into multiple 3-to-8-word
  fragments and are judged as a single fragment; containment still holds
  because the checks are capability-free.
