---
description: HTML evaluation reports derived from work evidence with model and process-change provenance.
visibility: public
delivered_by: [instrukt-first-build]
---
# Evaluation Report — Spec

## What it is

A developer examines delivery quality over time using the evidence already in
the work record. Evaluation accompanies recorded work and has no module switch
or opt-out setting. Reporting is an on-demand view of that evidence.

## Canonical fields

### Evidence source

`instrukt eval report` produces HTML from the record, whether the available
record is local, drawn from another project's reflected work or reconstructed
from GitHub. The report uses recorded evidence rather than a separate eval
log. There is only something to evaluate where work passes through the record.

`--since <date>` selects the history range. `--out <path>` selects the HTML
destination, defaulting to the current project’s `scratchpad/eval/index.html`.
With no recorded work, the report says so.

Model identity, version and eval fingerprint come from the stamps owned by
`local/projects/instrukt/spec/feature/work/stamps`. Report generation does not
invent missing observations or require conversation transcripts. Unreflected
local observations are available only where their record is available.

### Evaluation windows

The eval fingerprint identifies the process-document set selected for
measurement. Publication computes it from published global snippets marked `eval: true`,
hashed as committed raw content, and records it per published commit. Each
stamp copies the current fingerprint when written; recovered stamps therefore
retain their evaluation window without reconstructing a conversation.
A change to that set starts a distinct comparison window. Model and version
remain separate dimensions, so a model change is distinguishable from a
process-document change.

The report aggregates recorded work by project and evaluation window. It
shows the available review verdicts, rounds and findings, returns for repair,
and human intervention over time. Process changes are annotated with the
commits that changed the eval set. Missing evidence and incomplete history
remain visible limitations on a comparison.

The flavor’s activatable `sdlc/procedure/eval-report` runs the command, opens
or serves its HTML and explains changes alongside the commit SHAs that changed
the eval set. The report
does not add a statistical acceptance gate to delivery and does not require a
separate logging service.

### Use cases

#### UC-ER1: Compare delivery quality across process changes

```gherkin
  Given a developer has recorded work spanning different process fingerprints
  When they request an evaluation report
  Then they receive a readable report comparing the available outcomes over time
  And can distinguish process-document changes from changes in the model used
```
