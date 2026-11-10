---
description: Retained known-issue dispositions keep the log bug hunter from refiling a triaged signal while preserving other eligibility and deduplication checks.
---
# Known Issue Suppression — Spec

## What it is

A triaged known issue can be suppressed for the log bug hunter. The hunter
consults the project's retained suppressions before filing, so a repeated
known signal does not create another bug. Suppression does not settle a todo,
change a log record or replace the hunter's causal-evidence requirements.

## Canonical fields

A suppression identifies the project, the signal it covers and the reason for
its known-issue disposition. Retained suppressions are inspectable and can be
removed when that disposition changes. The hunter applies them within their
stated scope, before filing, alongside deduplication against active work.
Unrelated signals still pass through the ordinary eligibility and filing flow.

An unreadable suppression set leaves classification incomplete and is reported
under the hunter's recovery contract. It is never treated as an empty set.
The owning procedure is `sdlc/procedure/maintenance/log-bug-hunter`; the work
context retains known-issue dispositions independently of a scan checkpoint.

## Use cases

### UC-KS1: Stop refiling a triaged recurring signal

```gherkin
Given a project's recurring error has a retained known-issue suppression and reason
When the log bug hunter encounters that signal again
Then it applies the disposition without filing a duplicate bug
And unrelated signals still undergo the ordinary checks
When that suppression is removed
Then later matching signals are eligible for the ordinary filing flow again
```

## See Also

- [project/instrukt/spec/domain/work](../../domain/work.md)
- [sdlc/procedure/maintenance/log-bug-hunter](../../../../domain/sdlc/procedure/maintenance/log-bug-hunter.md)
- [sdlc/policy/todo-creation-discipline](../../../../domain/sdlc/policy/todo-creation-discipline.md)
