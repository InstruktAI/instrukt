//! Boundary fixtures shared by every CLI scenario.
#![allow(
    clippy::disallowed_methods,
    reason = "scenarios build their fixtures directly; the binary under test writes through the fence"
)]
#[path = "support/conversing.rs"]
pub mod conversing;
#[path = "support/documented.rs"]
pub mod documented;
#[path = "support/event_transport.rs"]
pub mod event_transport;
#[path = "support/flavor.rs"]
pub mod flavor;
#[path = "support/handler.rs"]
pub mod handler;
#[path = "support/history.rs"]
pub mod history;
#[path = "support/mapped.rs"]
pub mod mapped;
#[path = "support/mcp.rs"]
pub mod mcp;
#[path = "support/reviewed.rs"]
pub mod reviewed;
#[path = "support/runtime.rs"]
pub mod runtime;
#[path = "support/slack.rs"]
pub mod slack;
#[path = "support/t3.rs"]
pub mod t3;
#[path = "support/webhooks.rs"]
pub mod webhooks;
#[path = "support/work.rs"]
pub mod work;
#[path = "support/workflow.rs"]
pub mod workflow;
