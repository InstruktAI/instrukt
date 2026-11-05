---
eval: true
description: Bring a project's toolchain, quality configuration, and instrukt declarations into a working, self-contained contract.
visibility: 'public'
role: config-assessor
dispatch:
  agent: [claude, codex, gemini, deepseek]
  thinking_mode: med
---

# Code Config Assessment — Procedure

## Required reads

- [sdlc/design/project-toolchain](../design/project-toolchain.md)
- [sdlc/policy/language-policy-ownership](../policy/language-policy-ownership.md)
- [general/policy/human-contract](../../../general/policy/human-contract.md)
- [sdlc/policy/linting-requirements](../policy/linting-requirements.md)
- [general/policy/commits](../../../general/policy/commits.md)

## Goal

Make the project stand on its own toolchain while the declarations in its
`instrukt.yaml` tell the truth about it. Reassessment converges the project
again without undoing a defensible project choice.

## Preconditions

- Work from the project root and establish the current project state before
  changing it.
- Read the applicable language policies and the project's existing toolchain.
  The project's commands are its own; instrukt runs none of them and mandates
  no tool.

## Steps

1. Declare the project's install, lint, test, and format commands as `actions`
   in its `instrukt.yaml` (`test: make test`). Each action runs tools the
   project declares and provisions. A configured suite is run by test. Do not
   create an empty check or invent a suite to satisfy an action name.
2. Assess the project's own quality tools and configuration; instrukt ships no blessed quality template and owns no quality tooling. Preserve a deliberate, evidenced project choice. Resolve existing
   quality debt by fixing the code and configuration, not by disabling checks or
   adding suppressions to obtain a green result. A proposed relaxation of
   quality-defining configuration follows its independent review gate.
3. Leave no inert protection or configuration: each retained setting and check
   works, each displaced hook or file has its intended effect through the
   active toolchain, and secrets and generated output cannot be accidentally
   picked up by git.
4. Align the `actions` with the project's actual source and configured tests,
   and confirm with `instrukt config validate` that `instrukt.yaml` is valid.
   Confirm with `instrukt hooks status` that instrukt's hook sections are
   present, and preserve project-owned hook behavior beside them.
5. Prove the resulting contract from a clean project environment: the install
   action succeeds; the lint action, run unscoped, succeeds; and the test
   action succeeds when a suite is configured. Establish that the expected
   project-declared tools actually ran from the project's environment. Run the
   install action again when changed declarations require it.
6. Commit a complete, coherent assessment only after its applicable proof is
   green, then route the required independent review. If a human-owned
   consequence blocks completion, finish independent work and report the
   assessment as incomplete with the blocked consequence named.

## Outputs

- A self-contained project toolchain, live quality configuration, and accurate
  `instrukt.yaml` declarations with instrukt's hook sections in place.
- A final report with the install, unscoped lint, and applicable test outputs;
  evidence of which project tools ran and where they resolved; material
  technical decisions; and any unresolved blocker. A passing exit status
  without evidence that the intended checks ran is not completion.

## Recovery

- If an expected tool cannot be provisioned or a check is inert, repair its
  declaration or invocation and repeat the applicable proof. Do not substitute
  an ambient tool or weaken the quality gate.
- If a genuine human-owned decision is required, leave dependent work
  incomplete and identify the decision and consequence without treating it as
  approval of a technical mechanism.

## Discipline

The failure mode is a green assessment that runs the wrong tools, omits checks,
or leaves displaced protection inert. Ground the assessment in the project's
effective behavior and prove that behavior before declaring it done.
