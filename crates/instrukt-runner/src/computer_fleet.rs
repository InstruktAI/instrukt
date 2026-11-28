//! The computer fleet's query (`project/instrukt/spec/domain/runner`, "Fleet
//! query and membership"): a bounded request/reply over the cluster Redis
//! connection between the computers that share it, one queue per query kind
//! and per computer. Cluster Redis holds a request and its answer for their
//! lifetime and nothing of a kind's data beyond it.
//!
//! The query carries requests and answers; what they mean, which computers
//! answer a kind, and how a consumer words a missing answer are the kind's
//! owner's: merged history search's (`project/instrukt/spec/feature/runner/
//! history`) and the evaluation report's (`project/instrukt/spec/feature/
//! work/eval-report`). The name keeps it apart from fleet attention's fleet,
//! a project's active todos and their sessions.

mod _evaluation;
mod _failure;
mod _members;
mod _query;
mod _responder;

pub(crate) use _evaluation::{EvaluationHost, tick_step};
pub(crate) use _failure::{FailureFile, status as failure_status};

pub use _members::{LastSeen, Member, Members, MembersUnread};

pub use crate::transport::Reachability;
pub(crate) use _query::HISTORY;
pub use _query::{AskError, EVALUATION, Fleet, Kind, MAX_DEADLINE, Outcome, Unopened, open};
pub(crate) use _responder::{Answering, Responder, ResponderSpec, StartError};
