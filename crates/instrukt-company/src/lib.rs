//! The company context (`docs/project/spec/domain/company.md`): the connector
//! host and the conversations a company's people have with its agent. The
//! runner hosts it through `CompanyPorts` and resolves a conversation's
//! session through `conversation_assignment`; the CLI wires both, and calls
//! `speaker` for `instrukt mcp`.
pub mod connector_host;
pub mod conversations;
pub mod proposals;

/// How a connector kind is built once for a process, from its environment,
/// read through a lookup the host makes, and the outside gate the host made
/// for its hosting; a setting the kind refuses stops the runner at its start.
pub type BuildKind =
    fn(
        &dyn Fn(&str) -> Option<String>,
        instrukt_outside::Gate,
    ) -> Result<Box<dyn instrukt_connector::Kind>, instrukt_connector::ConnectorError>;
