---
description: HTML evaluation reports derived from work evidence with model and process-change provenance.
visibility: public
---
# Evaluation Report — Spec

## What it is

A developer examines delivery quality over time using the evidence already in
the work record. Evaluation accompanies recorded work and has no module switch
or opt-out setting. Reporting is an on-demand view of that evidence.

## Canonical fields

### Evidence source

<!-- planned-change:fleet-eval-aggregation -->
`instrukt eval report` produces HTML from this computer's work record. The
report uses recorded evidence rather than a separate eval log. There is only
something to evaluate where work passes through the record.
<!-- change:fleet-eval-aggregation -->
`instrukt eval report` produces HTML from the work record of this computer and
of each other computer the request names. The report uses recorded evidence
rather than a separate eval log. There is only something to evaluate where
work passes through the record.
<!-- /planned-change:fleet-eval-aggregation -->

`--since <date>` selects the history range. `--out <path>` selects the HTML
destination, defaulting to the current project’s `scratchpad/eval/index.html`.
With no recorded work, the report says so.

<!-- planned-change:fleet-eval-aggregation -->
One report covers every project known to this home's publication state,
grouped by project, with the current project first. Each project's record is
read from this computer's work record; the report reads nothing from GitHub.
The report takes no options beyond `--since` and `--out`.
<!-- change:fleet-eval-aggregation -->
One report covers every project known to the publication state of each
contributing computer, grouped by project, with the current project first. A
project recorded on several computers under the same project slug is one
project in the report. Each computer's part of a project's record is read
from that computer's own work record; the report reads nothing from GitHub,
from an agent runtime's own state or from history. Beyond `--since` and
`--out`, the report takes only the other computers to include, named by their
instrukt computer identities.
<!-- /planned-change:fleet-eval-aggregation -->

Model identity, version and eval fingerprint come from the stamps owned by
`project/instrukt/spec/feature/work/stamps`. Report generation does not
invent missing observations or require conversation transcripts.

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

<!-- planned:fleet-eval-aggregation -->
### Computers in a report

A report that names no other computer is this computer's alone: it reads as
described above, with its windows in time order and a restored eval set a
window of its own, and needs no cluster Redis.

Requests and answers travel through the runner's bounded fleet query
(`project/instrukt/spec/domain/runner`), which decides which named computers
answer. A computer answers over the cluster connection it already has, from
its own work record and publication state, applying the requested range,
whether or not history is switched on there; a named computer without a
cluster connection of its own does not answer.
For each project it counts its outcomes per eval set and model across all of
its stamps under that eval set, counting each todo, review and findings report
once per eval set. Its answer holds only the project slugs, eval sets and
model identities, those measures, the earliest and latest stamp time of each
project, eval set and model, the commits that computer records setting each
eval set, and the limitations it found. No stamp, artifact,
findings report or conversation leaves the computer, and no computer but the
requesting one writes a report.

A report that names another computer pools by eval set, whether or not any
named computer answers. Within each project it shows one window per eval set,
holding the outcomes of every answering computer, this one included: the
window spans the earliest to the latest stamp under that eval set across
those computers, its measures add per model and in the human row, and it
names the commits those computers record setting that eval set; where none
records one, the window carries the limitation that the commit setting it is
unknown. A todo's
record is held on one computer only
(`project/instrukt/spec/feature/work/state-database`); no todo, review or
findings report is counted twice. Stamps that copied no eval fingerprint form
one window per project, with the limitation that their process-document set is
unknown. The windows are listed by earliest stamp, and the report names no
process change and no restoration between them; a restored eval set's periods
form its one window. Model and version remain rows within each window, so a
model change stays distinguishable from a process-document change.

Every named computer appears in the report, either among the computers whose
answer it holds or as a limitation of the report with its reason, whatever
made its answer missing or unusable: one the fleet query reports as not
answering carries the reason the fleet query gives, and one whose answer this
computer cannot read is named as answering in a form it cannot read. Each
limitation a computer reports appears under that computer's identity, and the
report is written from the answers it has.
<!-- /planned:fleet-eval-aggregation -->

## Use cases

### UC-ER1: Compare delivery quality across process changes

```gherkin
  Given a developer has recorded work spanning different process fingerprints
  When they request an evaluation report
  Then they receive a readable report comparing the available outcomes over time
  And can distinguish process-document changes from changes in the model used
```

<!-- planned:fleet-eval-aggregation -->
### UC-ER2: Compare delivery quality across a developer's computers

```gherkin
  Given a developer records work on several of their computers
  When they request an evaluation report that includes those computers
  Then they receive one report giving each project one figure per process-document set, pooled from every answering computer
  And the report names the computers whose outcomes it holds
  And can distinguish process-document sets from the models used
  But when an included computer does not answer
  Then the report is still written from the computers that answered
  And it names the computer that did not answer as a limitation of the comparison
```
<!-- /planned:fleet-eval-aggregation -->

## See Also
- [project/instrukt/spec/domain/work](../../domain/work.md)
