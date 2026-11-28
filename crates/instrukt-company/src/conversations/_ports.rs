//! Company ports exposed to the runner.
use crate::connector_host::Connectors;
use instrukt_core::config::Admitted;
use instrukt_core::home::Home;
use instrukt_runner::interfaces::CompanyRelayPass;
use instrukt_runner::runtime::RuntimeAdapter;
use instrukt_runner::runtime::conversations::ConversationSource;
use jiff::Timestamp;
use std::sync::{Arc, Mutex};

use super::_intake::admitted_demand;
use super::_relay::{Accepted, RelayWorkers, relay, relay_state};
use super::_status::status_at;
use super::_store::outstanding;
use super::read_back;

/// The company's ports as the runner hosts it: built by the CLI with its
/// registry's connector builders, it builds the kinds when the runner starts
/// hosting and keeps the directory they make, and the answer pieces accepted
/// but not yet recorded, for its own relay.
pub struct Ports {
    builders: Vec<crate::BuildKind>,
    hosted: Mutex<Option<Arc<RelayWorkers>>>,
    accepted: std::sync::Arc<Accepted>,
}

impl Ports {
    pub fn new(builders: Vec<crate::BuildKind>) -> Self {
        Self {
            builders,
            hosted: std::sync::Mutex::default(),
            accepted: std::sync::Arc::default(),
        }
    }

    fn hosted(&self) -> Option<Arc<RelayWorkers>> {
        self.hosted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

fn company_error(error: impl std::fmt::Display) -> instrukt_runner::interfaces::CompanyError {
    instrukt_runner::interfaces::CompanyError(error.to_string())
}

impl instrukt_runner::interfaces::CompanyPorts for Ports {
    fn admitted_proposal(
        &self,
        home: &Home,
        root: &std::path::Path,
        head: &instrukt_core::ids::CommitSha,
    ) -> Result<bool, instrukt_runner::interfaces::CompanyError> {
        crate::proposals::admitted(home, root, head)
    }

    fn unestablished_proposals(
        &self,
        home: &Home,
        root: &std::path::Path,
    ) -> Result<
        Vec<instrukt_runner::interfaces::UnestablishedProposal>,
        instrukt_runner::interfaces::CompanyError,
    > {
        crate::proposals::unestablished(home, root)
    }

    fn proposal_store(
        &self,
        home: &Home,
        root: &std::path::Path,
    ) -> Result<
        instrukt_runner::interfaces::ProposalStoreCondition,
        instrukt_runner::interfaces::CompanyError,
    > {
        crate::proposals::condition(home, root)
    }

    fn maintain_read_back(
        &self,
        home: &Home,
        now: Timestamp,
    ) -> Result<(), instrukt_runner::interfaces::CompanyError> {
        read_back::maintain_at_start(home, now)
            .map_err(|error| instrukt_runner::interfaces::CompanyError(error.cause))
    }

    fn demand(&self, admitted: &Admitted) -> bool {
        !admitted_demand(admitted).is_empty()
    }

    fn start(
        &self,
        start: instrukt_runner::interfaces::CompanyStart<'_>,
    ) -> Result<
        Box<dyn instrukt_runner::interfaces::CompanySupervision>,
        instrukt_runner::interfaces::CompanyError,
    > {
        // One outside gate per hosting: every client of a connector in this
        // process shares its waits.
        let gate = instrukt_outside::Gate::new(
            std::sync::Arc::clone(&start.clock),
            std::sync::Arc::new(std::time::Instant::now),
            instrukt_outside::Report::log(),
        );
        let kinds = self
            .builders
            .iter()
            .map(|build| {
                let lookup = |name: &str| start.env.get(name).map(str::to_owned);
                build(&lookup, gate.clone())
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| instrukt_runner::interfaces::CompanyError(error.cause))?;
        let directory: std::sync::Arc<dyn Connectors> =
            std::sync::Arc::new(crate::connector_host::Directory::new(kinds, start.clock));
        *self
            .hosted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some(Arc::new(RelayWorkers::new(
                Arc::clone(&directory),
                start.wake.clone(),
                start.env.clone(),
            )));
        crate::connector_host::Connections::start(
            start.home,
            start.current,
            directory,
            std::sync::Arc::clone(&self.accepted),
            start.wake,
            start.supervise,
        )
        .map(|connections| {
            Box::new(connections) as Box<dyn instrukt_runner::interfaces::CompanySupervision>
        })
        .map_err(company_error)
    }

    fn relay(
        &self,
        home: &Home,
        admitted: &Admitted,
        source: &dyn ConversationSource,
        adapter: &dyn RuntimeAdapter,
        pass: CompanyRelayPass,
    ) -> Result<(), instrukt_runner::interfaces::CompanyError> {
        let Some(workers) = self.hosted() else {
            return Ok(());
        };
        relay(
            home,
            admitted,
            source,
            adapter,
            &workers,
            &self.accepted,
            pass,
        )
        .map_err(company_error)
    }

    fn outstanding(&self, home: &Home) -> Result<bool, instrukt_runner::interfaces::CompanyError> {
        outstanding(home).map_err(company_error)
    }

    /// Recorded on the clock the hosted connectors read, or the system's
    /// before any are hosted.
    fn relay_state(&self, home: &Home, failure: Option<&str>) {
        let now = self
            .hosted()
            .map_or_else(Timestamp::now, |workers| workers.connectors.now());
        relay_state(home, failure, now);
    }

    fn status(
        &self,
        home: &Home,
        now: Timestamp,
    ) -> Result<serde_json::Value, instrukt_runner::interfaces::CompanyError> {
        let status = status_at(home, now).map_err(company_error)?;
        serde_json::to_value(status).map_err(company_error)
    }
}

impl instrukt_runner::interfaces::CompanySupervision for crate::connector_host::Connections {
    fn check(&mut self) {
        crate::connector_host::Connections::check(self);
    }
}
