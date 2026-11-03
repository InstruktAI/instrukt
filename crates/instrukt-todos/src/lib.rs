//! The todos module: the work record (`docs/local/spec/domain/work.md`) and
//! the workflow engine (`docs/local/spec/domain/workflow.md`).
//!
//! Implements core's `TodoExistence` and `FlavorRoles` interfaces and the
//! runner's work and workflow interfaces; the CLI wires them when the module
//! is switched on.

pub mod work;
pub mod workflow;
