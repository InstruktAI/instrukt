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

One report covers every project known to this home's publication state,
grouped by project, with the current project first. Each project's record is
read from this computer's local record and, for items that are not on this
computer, from GitHub, so work done on another computer is reported here as
well; that is the record reconstructed from GitHub. The report is not a
listing: the developer asking for it is the interest that reads GitHub
(`project/instrukt/spec/feature/work/github-record`). A project whose record
cannot be read, for example without credentials or with GitHub unreachable,
is listed with that limitation rather than dropped. The report takes no
options beyond `--since` and `--out`.

Model identity, version and eval fingerprint come from the stamps owned by
`project/instrukt/spec/feature/work/stamps`. Report generation does not
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
and human intervention over time. Over time means the windows in order, each
with the time of its first and last stamp; the report adds no finer time
buckets. Windows follow time order, not fingerprint identity: a set that
changes and is later restored yields a window for each period, and the
restoration is a process change of its own, annotated with the commit that
restored it. Process changes are annotated with the commits that changed the
eval set; where the order of the recorded fingerprints cannot be established,
the report names no commit and shows the order as unknown. Missing evidence and incomplete history remain visible limitations
on a comparison.

Human acts stand outside the model dimension: each window has a row of its
own for them. A human approval given with `--session` is attributed to the
human, not to that session's model, and adds no unknown-model limitation.

The flavor’s activatable `sdlc/procedure/eval-report` runs the command, opens
or serves its HTML and explains changes alongside the commit SHAs that changed
the eval set. The report
does not add a statistical acceptance gate to delivery and does not require a
separate logging service.

### Outcome measures

- **Review verdicts and rounds:** the verdict stamps, approving and not
  approving, with one round per verdict. `needs_work` rounds are counted
  here and nowhere else.
- **Findings:** the distinct findings reports that verdicts bind.
- **Returns for repair:** kickbacks plus approvers' change requests. Review
  rounds that end `needs_work` are not counted again here.
- **Human intervention:** stamps marked `human_involved`, human approvals,
  change requests, and acts whose actor is a person or a GitHub actor.

## Use cases

### UC-ER1: Compare delivery quality across process changes

```gherkin
  Given a developer has recorded work spanning different process fingerprints
  When they request an evaluation report
  Then they receive a readable report comparing the available outcomes over time
  And can distinguish process-document changes from changes in the model used
```
