//! Read-only runner wake-up check (`requested-review`, Admission -> Delivery):
//! answers whether either the shared minute wake-up or the owned runner
//! service serves this computer, without mutating configuration or state.

use std::fmt;

use jiff::Timestamp;

use super::{
    _demand, InstallationError, Report, Request, ServiceState, ServiceStatus, TickMode,
    runner_service, ticks_status_with_demand,
};
use crate::install::WakeUp;

pub(crate) const INSTALL_DOOR: &str = "instrukt install";
pub(crate) const ENABLE_AND_INSTALL_DOOR: &str =
    "enable the todos or events module of a project on this computer, then run `instrukt install`";

/// Why no runner wake-up serves this computer for requested-review delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeupRefusal {
    /// The shared minute wake-up is demanded and its registration is missing
    /// or broken.
    RegistrationMissingOrBroken,
    /// A company home declares a connector and the owned runner service is
    /// unmet.
    RunnerServiceUnmet { cause: Option<String> },
    /// Neither the shared minute wake-up nor the owned runner service has
    /// demand on this computer.
    NoDemand,
}

impl fmt::Display for WakeupRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistrationMissingOrBroken => {
                f.write_str("the shared minute wake-up registration is missing or broken on this computer")
            }
            Self::RunnerServiceUnmet { cause: Some(cause) } => {
                write!(f, "the owned runner service is unmet on this computer ({cause})")
            }
            Self::RunnerServiceUnmet { cause: None } => {
                f.write_str("the owned runner service is unmet on this computer")
            }
            Self::NoDemand => f.write_str(
                "neither the shared minute wake-up nor the owned runner service has demand on this computer",
            ),
        }
    }
}

/// Whether a runner wake-up serves this computer to deliver verdict notices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerWakeup {
    /// Either the shared minute wake-up (registered or operator-scheduled) or
    /// the owned runner service serves this computer.
    Served { stale: bool },
    /// No wake-up serves this computer; names the semantic reason and
    /// sanctioned door.
    Refused {
        reason: WakeupRefusal,
        door: &'static str,
    },
}

fn decide(
    tick_mode: TickMode,
    service: Option<&ServiceStatus>,
    wakeup_stale: bool,
) -> RunnerWakeup {
    match tick_mode {
        TickMode::Operator | TickMode::Launchd | TickMode::Systemd | TickMode::Cron => {
            RunnerWakeup::Served {
                stale: wakeup_stale,
            }
        }
        TickMode::MissingOrBroken => {
            if service.is_some_and(|status| status.state == ServiceState::Running) {
                RunnerWakeup::Served {
                    stale: wakeup_stale,
                }
            } else {
                RunnerWakeup::Refused {
                    reason: WakeupRefusal::RegistrationMissingOrBroken,
                    door: INSTALL_DOOR,
                }
            }
        }
        TickMode::NoDemand => match service {
            Some(status) if status.state == ServiceState::Running => RunnerWakeup::Served {
                stale: wakeup_stale,
            },
            Some(status) if status.state == ServiceState::Unmet => RunnerWakeup::Refused {
                reason: WakeupRefusal::RunnerServiceUnmet {
                    cause: status.cause.clone(),
                },
                door: INSTALL_DOOR,
            },
            _ => RunnerWakeup::Refused {
                reason: WakeupRefusal::NoDemand,
                door: ENABLE_AND_INSTALL_DOOR,
            },
        },
    }
}

/// Inspects whether a runner wake-up serves this computer without writing
/// state or recording projects (`home.for_inspection()`).
pub fn runner_wakeup(
    request: &Request<'_>,
    now: Timestamp,
) -> Result<RunnerWakeup, InstallationError> {
    let home = request.home.for_inspection();
    let inspection = Request {
        home: &home,
        wake: WakeUp {
            home: &home,
            ..request.wake.clone()
        },
        check: true,
        module: None,
        conversations: false,
        ..request.clone()
    };
    let mut demand_report = Report::default();
    let demand = _demand::read(inspection.home, inspection.cwd, &mut demand_report)
        .map_err(|error| InstallationError::Demand(error.to_string()))?;
    let ticks = ticks_status_with_demand(&inspection, &demand, now)?;
    let service = if matches!(ticks.mode, TickMode::NoDemand | TickMode::MissingOrBroken) {
        let mut service_report = Report::default();
        runner_service(
            &Request {
                conversations: true,
                ..inspection.clone()
            },
            &demand,
            &mut service_report,
        );
        service_report.runner_service
    } else {
        None
    };
    let wakeup_stale = crate::ticks::status(ticks.last_wakeup, now).stale;
    Ok(decide(ticks.mode, service.as_ref(), wakeup_stale))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: ServiceState, cause: Option<&str>) -> ServiceStatus {
        ServiceStatus {
            state,
            cause: cause.map(str::to_owned),
            paths: Vec::new(),
        }
    }

    #[test]
    fn tick_modes_that_serve_propagate_staleness() {
        for mode in [
            TickMode::Operator,
            TickMode::Launchd,
            TickMode::Systemd,
            TickMode::Cron,
        ] {
            assert_eq!(
                decide(mode, None, false),
                RunnerWakeup::Served { stale: false }
            );
            assert_eq!(
                decide(mode, None, true),
                RunnerWakeup::Served { stale: true }
            );
        }
    }

    #[test]
    fn running_runner_service_serves_and_propagates_staleness_without_tick_demand() {
        let running = status(ServiceState::Running, None);
        assert_eq!(
            decide(TickMode::NoDemand, Some(&running), false),
            RunnerWakeup::Served { stale: false }
        );
        assert_eq!(
            decide(TickMode::NoDemand, Some(&running), true),
            RunnerWakeup::Served { stale: true }
        );
        assert_eq!(
            decide(TickMode::MissingOrBroken, Some(&running), true),
            RunnerWakeup::Served { stale: true }
        );
    }

    #[test]
    fn conversations_only_unmet_runner_service_refuses_with_install_door() {
        let unmet = status(ServiceState::Unmet, Some("service stopped"));
        assert_eq!(
            decide(TickMode::NoDemand, Some(&unmet), false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::RunnerServiceUnmet {
                    cause: Some("service stopped".into()),
                },
                door: INSTALL_DOOR,
            }
        );
    }

    #[test]
    fn missing_or_broken_tick_registration_refuses_with_install_door() {
        let none = status(ServiceState::NoDemand, None);
        assert_eq!(
            decide(TickMode::MissingOrBroken, Some(&none), false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::RegistrationMissingOrBroken,
                door: INSTALL_DOOR,
            }
        );
        let unmet = status(ServiceState::Unmet, Some("stopped"));
        assert_eq!(
            decide(TickMode::MissingOrBroken, Some(&unmet), false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::RegistrationMissingOrBroken,
                door: INSTALL_DOOR,
            }
        );
    }

    #[test]
    fn no_demand_from_either_ticks_or_runner_service_refuses_with_enable_door() {
        assert_eq!(
            decide(TickMode::NoDemand, None, false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::NoDemand,
                door: ENABLE_AND_INSTALL_DOOR,
            }
        );
        let no_demand = status(ServiceState::NoDemand, None);
        assert_eq!(
            decide(TickMode::NoDemand, Some(&no_demand), false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::NoDemand,
                door: ENABLE_AND_INSTALL_DOOR,
            }
        );
        let obsolete = status(ServiceState::Obsolete, Some("no connector"));
        assert_eq!(
            decide(TickMode::NoDemand, Some(&obsolete), false),
            RunnerWakeup::Refused {
                reason: WakeupRefusal::NoDemand,
                door: ENABLE_AND_INSTALL_DOOR,
            }
        );
    }
}
