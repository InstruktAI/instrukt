//! The connector interface of the company context
//! (`docs/project/spec/domain/company.md`): what a channel provider's
//! connector and the company's connector host exchange. It depends on nothing
//! of instrukt, so a connector builds against it alone.
//!
//! - [`Kind`] — one channel provider: its [`Declaration`] and the opening of
//!   one company's [`Connector`].
//! - [`Connector`] — one company's channel: what the connector contract of
//!   `docs/project/spec/feature/company/conversations.md` asks of it.
//! - [`Inbox`] — the host's side of [`Connector::listen`].
//! - [`Declaration`] and [`Declared`] — the fields a kind's `connectors.<kind>`
//!   entry takes, as data configuration validates against, and the values one
//!   company declared.
mod _connector;
mod _declaration;
mod _values;

pub use _connector::{Connector, Inbox, Kind, SecretResolver};
pub use _declaration::{Declaration, Declared, Field, Rule};
pub use _values::{
    Accompanying, Admission, Captured, Clock, ConnectorError, Earlier, Incoming, Limits, Place,
    Sender, Status,
};
