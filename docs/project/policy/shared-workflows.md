---
description: Instrukt's shared CI comes from its own organisation's repository, InstruktAI/workflows, so it composes from it at released versions.
---

# Shared Workflows — Policy

## Rules

- This project's shared workflows repository is `InstruktAI/workflows`, the
  InstruktAI organisation's own. It is therefore the within-organisation case of
  `sdlc/policy/ci-pipeline-conventions`: the project composes from it at a
  released version, and improvements and divergences go through it.

## Rationale

The global policy treats `InstruktAI/workflows` as another organisation's
repository for everyone outside InstruktAI. A reviewer of this project needs to
know that here it is the project's own; otherwise a released-version reference
reads as a movable reference to a stranger's code.

## Scope

This repository.

## Enforcement

- Reviewers apply the enforcement of `sdlc/policy/ci-pipeline-conventions` with
  `InstruktAI/workflows` as the existing path for generic pipeline logic.

## Exceptions

- None.
