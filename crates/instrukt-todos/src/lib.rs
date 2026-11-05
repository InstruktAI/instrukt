//! The todos module: the work record (`docs/project/spec/domain/work.md`) and
//! the workflow engine (`docs/project/spec/domain/workflow.md`).
//!
//! What each context implements, and for whom (wave 3):
//!
//! - [`work`] (the work builder): the local-leading record, its GitHub
//!   reflection and the `instrukt todo` record surfaces. Implements core's
//!   `TodoExistence` (`instrukt_core::interfaces`), the runner's work
//!   interfaces (`instrukt_runner::interfaces`: `Pickup`, `Claims`,
//!   `SessionBindings`, `Reflection`, `TaskFirings`, `AutomaticHolds`,
//!   `RetentionPurge`), and [`evidence::WorkEvidence`] for workflow. Its
//!   finalize-ready stamp appends through the runner's
//!   `instrukt_runner::events::EventLog`, handed in by the CLI.
//! - [`workflow`] (the workflow builder): the pure engine (wave 1), the
//!   fact provider over [`evidence::TodoEvidence`], `instrukt todo
//!   prepare|work`, and the implementations of core's `FlavorRoles` and the
//!   runner's `WorkflowAssignments`. It records claims through the runner
//!   interface `Claims`, implemented by work.
//! - [`evidence`]: the work → workflow evidence contract both build
//!   against.
//!
//! The CLI wires every implementation (`crates/instrukt-cli/src/wiring.rs`)
//! only when `modules.todos` is on.

pub mod eval;
pub mod evidence;
pub mod work;
pub mod workflow;
