//! The runner context: module-aware dispatch, standing agents, event
//! sources, the sessions facade and task timing
//! (`docs/project/spec/domain/runner.md`). Shared by the `todos` and `events`
//! modules; depends on core only.
//!
//! - [`interfaces`] — what the runner reads from work and workflow and
//!   writes through work's command contract. The todos crate implements
//!   them; the CLI wires them in when `todos` is on.
//! - [`events`] — the event vocabulary and the event-log append the runner
//!   implements and work's finalize-ready stamp uses.
//!
//! - [`runner`] — `tick` and `run`: the lock, the wake-up and the
//!   projects considered.
//! - [`sessions`] and [`launch`] — the sessions facade over the runtime
//!   mapping, and procedure launches.
//! - [`event_log`], [`sources`], [`ci`] and [`standing`] — the event log,
//!   the event sources and standing-agent delivery.
//! - [`dispatch`], [`agents`] and [`timing`] — todo pickup, the agent
//!   order and task due times.
//! - [`anomaly`], [`ticks`] and [`install`] — notification, wake-up
//!   accounting and the OS scheduler registration.
//! - [`fleet_attention`] — the conditions of a project's active work that
//!   need attention from outside one todo, and its attention episode.
//! - [`reclamation`] — worktree reclamation: declared regenerable output
//!   from idle todo worktrees, and delivered todos' worktrees.
//! - [`thread_retention`] — archiving long-quiet finished runtime threads.
#![cfg_attr(
    test,
    allow(
        clippy::disallowed_methods,
        reason = "unit tests build their scratch trees directly"
    )
)]

pub mod agents;
pub mod anomaly;
pub mod ci;
pub mod consumers;
pub mod control;
pub mod dispatch;
pub mod event_log;
pub mod events;
pub mod executions;
pub mod fleet_attention;
pub mod history;
pub mod install;
pub mod interfaces;
pub mod keyed;
pub mod launch;
pub mod process;
pub mod reclamation;
pub mod requested_review;
pub mod review;
pub mod runner;
pub mod runtime;
pub mod sessions;
pub mod sources;
pub mod standing;
pub mod state;
pub mod thread_retention;
pub mod ticks;
pub mod timing;
pub mod transport;

pub mod webhooks;

mod _attention;
pub mod inputs;

pub use _attention::deliver_approval_attention;

pub mod assignments;

pub mod mcp;
pub(crate) mod relay;

pub mod installation;
