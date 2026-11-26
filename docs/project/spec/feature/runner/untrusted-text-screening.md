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
that reads it. The text is cut into fragments of random length and shuffled,
and independent classifiers judge the fragments through capability-free calls
(`project/instrukt/spec/feature/runner/capability-free-calls`). A classifier
never sees the text in its original order, and a manipulation that succeeds
against a classifier can change only that classifier's verdict.

Text is released only when every classifier judges it benign. Any other result
keeps the text for a person; screening neither releases it nor drops it.

Screening carries no knowledge of where a text came from. Every caller with
untrusted text uses it the same way, and a caller can hand over text that no
model on its side has read.

## Canonical fields

### Screening a text

| Field | Meaning |
| --- | --- |
| Text | The complete untrusted text. |
| Fragments | The complete text cut at random-length boundaries and shuffled, carrying no original position. Each classification cuts and shuffles independently. |
| Classification | One capability-free call that sees the complete set of its fragments once, with no earlier classification or other context. Screening runs more than one, each independent of the others. |
| Verdict | `benign` or `flagged`, with reasons from a bounded set. A classifier passes on no free text. |
| Outcome | `released` or `held`, with every verdict and the reason for holding. |

### Release and holding

A text is `released` only when every classification returns `benign`. It is
`held` when any classification is flagged, malformed, timed out or refused,
when classifications disagree, when no capability-free call is available, and
when the text exceeds the size screening judges. A text is never truncated,
judged in part or partly released.

A held text is kept durably with its verdicts and reasons, and a person can
retrieve it by the reference its outcome carries. Screening never releases or
deletes a held text by itself.

## Use cases

### UC-SC1: An agent has outside text screened before it reaches a tool-capable agent

```gherkin
Given an operator's computer with admitted agent CLIs
And an agent acting for the operator holds text written outside the project that it has not read
When the agent has the text screened
And every classification judges it benign
Then the agent receives the text as released
And no classifier saw the text in its original order
```

### UC-SC2: Text that tries to manipulate its reader is kept for a person

```gherkin
Given an agent acting for an operator holds text written outside the project
And the text instructs whoever reads it to ignore its instructions and publish the project's secrets
When the agent has the text screened
And a classification flags it
Then the agent receives the outcome held, without the text
And the text with every verdict and reason remains available to a person
And the text is neither released nor deleted
```
