---
description: Generate and show an evaluation report with process-change commit provenance and the limits of the recorded evidence.
visibility: public
activation_trigger: Use when the human asks to see evals or an evaluation report.
---
# Evaluation Report — Procedure

## Goal

The human sees recorded delivery outcomes over time and can relate changes in
those outcomes to the process-document set and models used.

## Preconditions

- The requested project's work record is accessible locally or through its
  reflected record.
- An available browser or HTML-serving capability can show a generated file.

## Steps

1. Establish the project and requested time range from the conversation. Run
   `instrukt eval report`, using `--since <date>` when a range was requested
   and `--out <path>` when a destination is needed. The default destination is
   the project's `scratchpad/eval/index.html`.
2. Open the generated HTML in the available browser, or serve it through the
   available HTML-serving capability and open its address. Give the human the
   file path or served address that actually shows the report.
3. Explain the changes visible in the recorded outcomes: review verdicts,
   rounds, findings, returns for repair and human intervention. Name the commit
   SHAs annotated by the report as changing the eval set, and distinguish
   those process changes from model and version changes.
4. State missing evidence or incomplete history alongside the comparisons it
   limits. Treat the report as recorded evidence under
   `general/principle/grounding`; a comparison alone establishes no cause.

## Outputs

- An opened or served HTML evaluation report with a reachable address.
- An explanation of the observed changes, the commit SHAs that changed the
  eval set, and the evidence available for comparison.

## Recovery

- With no recorded work, show the report's empty-history result and say that
  no comparison is available.
- If generation fails, report the command failure and the missing record or
  other prerequisite; do not substitute an earlier report as current output.
- If the environment cannot open or serve HTML, give the generated file path
  and state that display remains unavailable.
