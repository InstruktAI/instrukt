//! Requested review (`project/instrukt/spec/feature/runner/requested-review`):
//! an authoring session asks for the independent review of an exact
//! committed worktree head by naming its coordinates only, and the runner
//! admits the ask, renders the reviewer's assignment and starts or sends to
//! the head's requested reviewer itself.

#[allow(
    dead_code,
    reason = "the checks are called by the ask, which arrives with the start"
)]
mod _admission;
#[allow(
    dead_code,
    reason = "the ask renders its assignment with it, and arrives with the start"
)]
mod _rendering;
#[expect(
    dead_code,
    reason = "the ask that writes and reads it arrives with the start"
)]
mod _store;

pub use _admission::{Coordinate, Coordinates, FormRefusal, Kind, RawCoordinates, SourcePointer};

/// The admission check that refused an ask
/// (`requested-review`, Admission).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    Placement,
    Cleanliness,
    Range,
    ReviewState,
    Request,
    Outstanding,
    Runtime,
    Procedure,
}

/// Why an ask was not admitted or not accepted.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// A value does not have its coordinate's form.
    #[error(transparent)]
    Form(#[from] FormRefusal),
    /// An admission check refused it; the message names what was found and
    /// the door.
    #[error("{message}")]
    Refused { check: Check, message: String },
    /// What admission or delivery reads could not be read.
    #[error("{0}")]
    Failed(String),
}
