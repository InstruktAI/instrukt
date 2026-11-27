---
description: Untrusted text is judged for manipulation by independent capability-free classifiers that see it only as shuffled fragments, and is released only when every judgment is benign; any other text is kept for a person.
delivered_by:
  - capability-free-model-call
---
# Untrusted-Text Screening — Spec

## What it is

Untrusted text is text written by people outside the project, such as the
description of an outside contributor's pull request. Before such text reaches
a tool-capable agent, screening judges whether it tries to manipulate the model
that reads it. The text is cut into shuffled fragments of 3 to 8 words,
and independent classifiers judge the fragments through capability-free calls
(`project/instrukt/spec/feature/screening/capability-free-calls`). A classifier
never sees the text in its original order, and a manipulation that succeeds
against a classifier can change only that classifier's verdict.

Text is released only when every classifier judges it benign. Any other
screening result keeps the text for a person; screening neither releases it nor
drops it. A held text never reaches an agent working from instrukt's briefs; a
person, or an agent the operator drives directly, which is the operator's own,
retrieves it. The rule keeps instrukt from handing held outside text to
brief-governed agents; it is no defence against agents, which are trusted.

Screening carries no knowledge of where a text came from. Every caller with
untrusted text uses it the same way, and a caller can hand over text that no
model on its side has read.

## Canonical fields

### Screening a text

| Field | Meaning |
| --- | --- |
| Text | The complete untrusted text. |
| Fragments | The complete text cut into fragments of 3 to 8 words, of random length within that range, and shuffled, carrying no original position. |
| Classification | One capability-free call that judges for manipulation and sees the complete set of its fragments once, with no earlier classification or other context. Screening runs more than one, each independent of the others. |
| Verdict | `benign` or `flagged`, with reasons from a bounded set. A classifier passes on no free text. |
| Outcome | `released` with the text, `held` with every verdict, the reason for holding and a reference, or `refused` when an endpoint prerequisite is missing or unresolvable. While the endpoint is unavailable the screening is `waiting`, a state in which it returns no outcome yet. A held outcome never carries the text. |

### Release and holding

A text is `released` only when every classification returns `benign`. It is
`held` when any classification is flagged, when an answer is not of the
verdict's shape or was cut off, when the endpoint rejects a classification's
request as one it cannot serve, when classifications disagree, and when the
text is over the size screening judges. A text is never truncated, judged in
part or partly released.

While a classification waits on an unavailable endpoint
(`project/instrukt/spec/feature/screening/capability-free-calls`), the
screening is `waiting`: it has no outcome yet and the text is neither released
nor held. instrukt, not the caller, tries it again through the outside gate
(`project/instrukt/design/architecture/outside-gate`), and the caller receives
the outcome, released or held, once the endpoint answers, without asking again.
Before any text is taken, any missing or unresolvable endpoint prerequisite —
an unconfigured or incomplete endpoint, or missing, blank, unreadable or
insufficiently protected referenced API key secret material
(`project/instrukt/spec/feature/configuration/owned-files`) — is `refused` at
entry, naming what is missing and the door to supply it
(`project/instrukt/policy/checks-and-guards`), and nothing is released.

A defect, a failure that is neither an entry refusal, an unavailable endpoint
nor one of the post-call results above, such as a malformed request or a client
or protocol failure (`project/instrukt/policy/checks-and-guards`), fails the
screening: it releases nothing and records no verdict or held text.

A held text is kept durably with its verdicts and reasons. An agent working
from instrukt's briefs receives its reference, verdicts and reasons, never its
text, so retrieval never releases what screening held. A person, or an agent
the operator drives directly, retrieves the text; caller recognition belongs
to `project/instrukt/spec/feature/configuration/roles-and-capabilities`.
Screening never releases or deletes a held text by itself.

## Use cases

### UC-SC1: An agent has outside text screened before it reaches a tool-capable agent

```gherkin
Given an operator has configured an endpoint for capability-free calls
And an agent acting for the operator holds text written outside the project that it has not read
When the agent has the text screened
And every classification judges it benign
Then the agent receives the text as released
And no classifier saw the text in its original order
```

### UC-SC2: Text that tries to manipulate its reader is kept for a person

```gherkin
Given an agent working from instrukt's brief holds text written outside the project
And the text instructs whoever reads it to ignore its instructions and publish the project's secrets
When the agent has the text screened
And a classification flags it
Then the agent receives the outcome held, without the text
And no agent working from instrukt's briefs can retrieve the text
And the operator, or an agent the operator drives directly, can read the text with every verdict and reason
And the text is neither released nor deleted by screening
```
