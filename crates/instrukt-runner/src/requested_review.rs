//! Requested review (`project/instrukt/spec/feature/runner/requested-review`):
//! an authoring session asks for the independent review of an exact
//! committed worktree head by naming its coordinates only, and the runner
//! admits the ask, renders the reviewer's assignment and starts or sends to
//! the head's requested reviewer itself.

#[expect(
    dead_code,
    reason = "the ask that writes and reads it arrives with the start"
)]
mod _store;
