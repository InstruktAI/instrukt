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
Nonempty text of six or more words is cut into random fragments of 3 to 8 words
and shuffled without original positions; nonempty text of one to five words is
sent as a single fragment rather than rejected for its length. More than one
independent capability-free check
(`project/instrukt/spec/feature/screening/capability-free-calls`, using global
`llm.screening`) judges whether the fragments look manipulative or malicious. A
manipulation that succeeds against a check can change only that check's
verdict.

Screening is stateless and stores no copy of the text locally. `instrukt
is-safe` returns a pure boolean verdict: safe only when every check judges
benign, or unsafe with a short reason otherwise, without echoing the text back
and without producing a hash or stored reference.

Screening carries no knowledge of where a text came from. Standard input is the
primary command input so untrusted text need not touch disk, with an optional
file path for callers that supply a temporary file; internal callers pass text
in memory.

Screening stores no durable verdict history of its own and emits standard
command diagnostics (`project/instrukt/spec/feature/configuration/logging`).

## Canonical fields

### Screening a text

| Field | Meaning |
| --- | --- |
| Text | The complete nonempty untrusted text, supplied on standard input (primary), from an optional file path, or in memory by an internal caller. |
| Fragments | For nonempty texts of six or more words, the complete text cut into fragments of random length from 3 to 8 words and shuffled with no original positions; for nonempty texts of one to five words, the complete text as a single fragment. Separate checks need not receive distinct shuffles. |
| Check | One capability-free call (`project/instrukt/spec/feature/screening/capability-free-calls`, against `llm.screening`) asking whether the fragments look manipulative or malicious, with no earlier check or other context. Screening runs more than one independent check. |
| Verdict | A simple `benign` or `flagged` judgment; when `flagged`, it carries a bounded short `reason` summarizing the concern without quoting the screened text. |
| Outcome | `safe` (exit 0, `true`, or `{"safe": true}` with `--json`), `unsafe` with a short reason (non-zero exit, `false`, or `{"safe": false, "reason": "..."}` with `--json`), `refused` at entry (non-zero exit with cause, producing no safety verdict), or `waiting` while the endpoint is unavailable (returning the wait when a caller's wait bound expires). No outcome echoes the text, emits a hash or returns a stored reference. |

### Safety decision, waiting and failures

Screening maps its inputs and the outcomes of
`project/instrukt/spec/feature/screening/capability-free-calls` (for
`llm.screening`) onto its stateless contract:

- **Safe.** Text is judged `safe` only when every independent check returns
  `benign`.
- **Unsafe.** Text is judged `unsafe`, with a short reason, when any check
  returns `flagged`, when checks disagree, when a check's answer is not a valid
  verdict, or when a check ends in a post-call refusal from
  `capability-free-calls` (cut off or rejected by the endpoint). Text is never
  truncated or judged in part.
- **Entry refusal.** Missing, unreadable or non-UTF-8 CLI input (`<file>` or
  standard input), empty or whitespace-only input (zero words), or an entry
  refusal from `capability-free-calls` refuses screening at entry (non-zero
  exit naming the cause), makes no model call, emits neither `safe` nor
  `unsafe`, and is distinct from a code defect.
- **Waiting.** While a check is `waiting` under `capability-free-calls`,
  screening is `waiting` and returns neither `safe` nor `unsafe`: unbounded
  callers wait until the endpoint answers, while bounded-wait callers receive
  the wait once their bound expires.
- **Defect.** A defect returned by `capability-free-calls` fails screening
  without retrying and never reports `safe`.

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
- Texts of one to five words cannot be shuffled into multiple 3-to-8-word
  fragments and are judged as a single fragment; containment still holds
  because the checks are capability-free.
- A flagged verdict's short `reason` is authored by a model that read untrusted
  input. Even though it is bounded and instructed not to quote the screened
  text, it remains a residual second-order prompt-injection surface:
  tool-capable callers must treat `reason` as untrusted data, never as
  instructions (`project/instrukt/spec/feature/screening/capability-free-calls`).
