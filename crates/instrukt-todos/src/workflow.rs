//! The workflow context: derivation of obligations from configured definitions and facts, and the integration gate (`docs/local/spec/domain/workflow.md`).
//!
//! The engine is one pure capability: given a loaded [`Flavor`] (definitions,
//! actions, effects, roles and templates from a published domain's
//! `workflow/` directory) and a [`Subject`]'s typed [`Facts`],
//! [`Flavor::evaluate`] derives the state, the selected transition, the action
//! with its rendered command and instructions, and the effects still owed. It
//! stores nothing, executes nothing and reads no clock, repository, network
//! or runtime; configuration paths in placeholders are answered through the
//! caller's [`ConfigLookup`].

mod _condition;
mod _derive;
mod _facts;
mod _finding;
mod _flavor;
mod _names;
mod _render;
mod _schema;
#[cfg(test)]
mod _sdlc_flavor;
#[cfg(test)]
mod _testing;

pub use _derive::{
    Derivation, EvaluationError, Obligation, OwedEffect, Responsible, SelectedTransition, Subject,
};
pub use _facts::{BoundSession, FactError, FactName, FactType, FactValue, Facts};
pub use _finding::{Finding, Problem};
pub use _flavor::{Flavor, FlavorError};
pub use _names::{ActionId, EffectName, FlowName, StateName};
pub use _render::{ConfigLookup, ConfigLookupError, RenderFailure, RenderFinding, Rendering};
pub use _schema::WorkflowSchema;
