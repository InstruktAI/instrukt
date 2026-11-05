//! The repository context: adoption, hook sections and attributed commits
//! (`docs/project/spec/domain/repository.md`).
//!
//! - [`init`] adopts a repository: project layer, docs folders, master
//!   and hook sections, never overwriting a file.
//! - [`install_hooks`], [`remove_hooks`] and [`hooks_status`] manage
//!   instrukt's one marked section per hook file and touch nothing else.
//! - [`run_hook`] is what a section calls: it starts what acts on the hook
//!   point and returns without waiting; [`run_hook_worker`] is the detached
//!   process that waits for the sync it started and records the outcome.
//! - [`commit`] makes an attributed commit; [`stamp()`] records a
//!   commit-review verdict as a git note and pushes the notes ref;
//!   [`read_review_notes`] fetches it and reads the verdicts.

mod _adoption;
mod _commit;
mod _contain;
mod _error;
mod _git;
mod _hook;
mod _hooks;
mod _notes;
mod _run;
mod _section;
mod _stamp;

pub use _adoption::{
    GENERATED_FILE, InitPart, InitReport, LOCAL_BASELINE, MASTER_FILE, PartOutcome, init,
};
pub use _commit::{CommitRequest, OWNED_TRAILERS, commit};
pub use _error::RepositoryError;
pub use _git::HooksDir;
pub use _hook::Hook;
pub use _hooks::{
    HookChange, HookStatus, HooksReport, HooksStatus, SectionChange, SectionStatus, hooks_status,
    install_hooks, remove_hooks,
};
pub use _notes::{NotesFetch, NotesPush, PendingReason, REVIEW_NOTES_REF};
pub use _run::{
    GenerationOutcome, HOOK_WORKER_COMMAND, HookInvocation, HookRunRecord, SyncOutcome, last_run,
    run_hook, run_hook_worker,
};
pub use _section::{SECTION_END, SECTION_START, SectionError, hook_run_line};
pub use _stamp::{
    CommitReview, ReviewNotes, Settlement, Stamp, StampOutcome, StampTime, TimedStamp, Verdict,
    read_review_notes, stamp,
};
