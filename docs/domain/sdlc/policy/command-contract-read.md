---
description: 'A session started into a procedure reads the contract of each command that procedure uses before its assigned work; a session reading the procedure inline keeps its own tool context.'
visibility: 'public'
---

# Command Contract Read — Policy

## Rules

- When a procedure that requires this policy is your session's launch
  assignment, read the contract of each command it uses with
  `instrukt <command> -h` before starting the assigned work.
- When you read that procedure inline, during other work, keep your current
  session's tool context.

## Rationale

- A command's `-h` is its current contract; a session that acts on a command
  from memory or from the procedure's prose alone misuses whatever changed
  since.
- A session reading a procedure inline already holds the tools and context of
  its own work; re-establishing a launch context there would displace it.

## Scope

- Every session started into, or reading, a procedure that requires this
  policy.

## Enforcement

- A reviewer of a session's transcript treats a command used before its
  contract was read, in a launch assignment, as a finding.

## Exceptions

- None.
