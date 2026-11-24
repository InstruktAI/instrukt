//! Automatic review's requests as the runner holds them
//! (`project/instrukt/spec/feature/runner/pull-request-review`, "One review
//! per commit"): what work reads when it admits a verdict. Each answer comes
//! from delivery's own records, the selected runtime or the history store,
//! read without a lock.

mod _conversation;
mod _requests;

pub use _conversation::{Conversation, ConversationHolder, Message, conversation};
pub use _requests::{
    Disposition, Request, Standing, TurnOpening, requests, turn_opening, turn_opening_with,
};

/// A review answer could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ReviewReadError {
    #[error(transparent)]
    State(#[from] crate::state::StateError),
    #[error("a review request's record is unreadable: {0}")]
    Record(String),
}
