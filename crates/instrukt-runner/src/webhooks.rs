//! Signed, project-scoped loopback intake. Acceptance ends at cluster append.
mod _github;
mod _protocol;

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{Configuration, Env, Module, WebhookProtocol, webhook_domain};
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::machine::machine_id;
use instrukt_core::publication::PublishedRecord;
use jiff::Timestamp;
use sha2::{Digest, Sha256};
use tiny_http::{Method, Request, Response, Server};

use crate::events::{
    EventEnvelope, EventHome, EventHomeOwner, EventRef, ReferenceKind, occurrence_id,
};
use crate::state::RunnerState;
use crate::transport::ScopedTransport;

pub use _github::{GithubView, github_view};

const CLAIM_TTL: u64 = 3 * 24 * 60 * 60;

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Machine(#[from] instrukt_core::machine::MachineError),
    #[error("this computer is not the configured ingress")]
    Placement,
    #[error("no enabled registered project declares a readable webhook source")]
    NoSources,
    #[error(transparent)]
    Registration(#[from] instrukt_core::publication::PublicationStateError),
    #[error("project {project}: {detail}")]
    Declaration {
        project: ProjectSlug,
        detail: String,
    },
    #[error("webhook listener failed: {0}")]
    Listen(std::io::Error),
}

/// Internal service entry for a direct, placement-checked loopback listener.
pub fn serve(
    home: &Home,
    env: &Env,
    clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
) -> Result<(), WebhookError> {
    serve_with_socket(home, env, clock, false)
}

pub fn serve_with_socket(
    home: &Home,
    env: &Env,
    clock: std::sync::Arc<dyn Fn() -> Timestamp + Send + Sync>,
    socket_activation: bool,
) -> Result<(), WebhookError> {
    serve_prepared(ServeRequest {
        home,
        env,
        clock,
        socket_activation,
        prepared: None,
    })
}
pub struct PreparedReceiver {
    listener: TcpListener,
    installed_port: Option<u16>,
}
pub struct ServeRequest<'a> {
    pub home: &'a Home,
    pub env: &'a Env,
    pub clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    pub socket_activation: bool,
    pub prepared: Option<PreparedReceiver>,
}
pub fn serve_prepared(request: ServeRequest<'_>) -> Result<(), WebhookError> {
    let ServeRequest {
        home,
        env,
        clock,
        socket_activation,
        prepared,
    } = request;
    let inspection = home.for_inspection();
    let computer = machine_id(home)?;
    let ingress = Configuration::load_global(&inspection)?.ingress()?;
    if ingress.computer != computer {
        return Err(WebhookError::Placement);
    }
    let mut demand = false;
    let mut invalid = None;
    for record in PublishedRecord::load_all(home)? {
        let declaration = (|| -> Result<bool, String> {
            let config = Configuration::load(&inspection, Some(&record.checkout))
                .map_err(|_| "cannot read webhook configuration".to_owned())?;
            let sources = config.webhooks().map_err(|error| error.to_string())?;
            if !config
                .module_enabled(Module::Events)
                .map_err(|_| "cannot read modules.events".to_owned())?
            {
                return Ok(false);
            }
            for source in sources.values() {
                source.resolve(home).map_err(|error| error.to_string())?;
            }
            Ok(!sources.is_empty())
        })();
        match declaration {
            Ok(enabled) => demand |= enabled,
            Err(detail) => {
                tracing::warn!(event = "webhook_declaration_unreadable", project = %record.project, error = %detail);
                invalid = Some(WebhookError::Declaration {
                    project: record.project,
                    detail,
                });
            }
        }
    }
    if !demand {
        return Err(invalid.unwrap_or(WebhookError::NoSources));
    }
    let prepared = if socket_activation {
        Some(match prepared {
            Some(prepared) => prepared,
            None => prepare_activation(home)?,
        })
    } else {
        None
    };
    let installed_port = prepared.as_ref().and_then(|p| p.installed_port);
    let listener = match prepared {
        Some(prepared) => prepared.listener,
        None => TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, ingress.port))
            .map_err(WebhookError::Listen)?,
    };
    let address = listener.local_addr().map_err(WebhookError::Listen)?;
    if address.ip() != std::net::Ipv4Addr::LOCALHOST
        || address.port() != installed_port.unwrap_or(ingress.port)
    {
        return Err(WebhookError::Listen(std::io::Error::other(
            "service-manager socket does not match configured loopback ingress port",
        )));
    }
    listener
        .set_nonblocking(false)
        .map_err(WebhookError::Listen)?;
    let server = Server::from_listener(listener, None)
        .map_err(|_| WebhookError::Listen(std::io::Error::other("cannot create HTTP server")))?;
    tracing::info!(
        event = "webhooks_listening",
        port = ingress.port,
        "signed webhook receiver ready on loopback"
    );
    loop {
        // A valid placement change withdraws this receiver even when no requests arrive.
        match Configuration::load_global(&inspection).and_then(|config| config.ingress()) {
            Ok(current) if current.computer != computer => return Err(WebhookError::Placement),
            Err(error) => tracing::warn!(event = "webhook_configuration_invalid", error = %error),
            _ => {}
        }
        let Some(mut request) = server
            .recv_timeout(Duration::from_secs(1))
            .map_err(WebhookError::Listen)?
        else {
            continue;
        };
        let response = receive(home, env, computer.as_str(), &mut request, clock());
        if let Err(error) =
            request.respond(Response::from_string(response.1).with_status_code(response.0))
        {
            tracing::warn!(event = "webhook_response_failed", error = %error);
        }
    }
}

/// Apply a proven effective invocation before logging or receiver configuration reads.
/// Exec retains the listening descriptor and leaves the manager's socket intact.
pub fn prepare_activation(home: &Home) -> Result<PreparedReceiver, WebhookError> {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let listener = managed_listener()?;
    let address = listener.local_addr().map_err(WebhookError::Listen)?;
    if !address.ip().is_loopback() {
        return Err(WebhookError::Listen(std::io::Error::other(
            "activation socket must be loopback",
        )));
    }
    let effective = crate::installation::_launch::owned(home, address.port())
        .map_err(|e| WebhookError::Listen(std::io::Error::other(e)))?;
    let installed_port = effective.as_ref().map(|_| address.port());
    if let Some((effective, digest)) = effective
        && !effective.applied(&digest)
    {
        rustix::io::fcntl_setfd(&listener, rustix::io::FdFlags::empty())
            .map_err(|e| WebhookError::Listen(e.into()))?;
        let error = std::process::Command::new(&effective.executable)
            .args(["webhooks", "serve", "--socket-activation"])
            .envs(&effective.environment)
            .env("INSTRUKT_RECEIVER_FD", listener.as_raw_fd().to_string())
            .env("INSTRUKT_RECEIVER_APPLIED", digest)
            .exec();
        return Err(WebhookError::Listen(error));
    }
    Ok(PreparedReceiver {
        listener,
        installed_port,
    })
}
fn managed_listener() -> Result<TcpListener, WebhookError> {
    let binding = if let Ok(fd) = std::env::var("INSTRUKT_RECEIVER_FD") {
        let fd = fd
            .parse::<i32>()
            .map_err(|e| WebhookError::Listen(std::io::Error::other(e)))?;
        service_binding::Binding::FileDescriptor(fd)
    } else {
        let name = if std::env::var_os("LISTEN_FDS").is_some() {
            if std::env::var("LISTEN_PID").ok().as_deref() != Some(&std::process::id().to_string())
                || std::env::var("LISTEN_FDS").ok().as_deref() != Some("1")
                || std::env::var("LISTEN_FDNAMES").ok().as_deref() != Some("instrukt")
            {
                return Err(WebhookError::Listen(std::io::Error::other(
                    "invalid service-manager socket handoff",
                )));
            }
            "fd://"
        } else {
            "fd://instrukt"
        };
        name.parse::<service_binding::Binding>()
            .map_err(|e| WebhookError::Listen(std::io::Error::other(e)))?
    };
    match service_binding::Listener::try_from(binding).map_err(WebhookError::Listen)? {
        service_binding::Listener::Tcp(listener) => Ok(listener),
        service_binding::Listener::Unix(listener) => {
            let descriptor: std::os::fd::OwnedFd = listener.into();
            Ok(TcpListener::from(descriptor))
        }
        _ => Err(WebhookError::Listen(std::io::Error::other(
            "service manager did not pass a TCP listener",
        ))),
    }
}

fn route(url: &str) -> Option<(ProjectSlug, String)> {
    let (slug, domain) = url
        .strip_prefix("/hooks/inbound/project/")?
        .split_once('/')?;
    if !webhook_domain(domain) {
        return None;
    }
    Some((ProjectSlug::parse(slug).ok()?, domain.to_owned()))
}

fn header(request: &Request, name: &str) -> Option<String> {
    let mut headers = request
        .headers()
        .iter()
        .filter(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name));
    let value = headers.next()?.value.as_str().to_owned();
    headers.next().is_none().then_some(value)
}

fn receive(
    home: &Home,
    env: &Env,
    computer: &str,
    request: &mut Request,
    now: Timestamp,
) -> (u16, String) {
    let Some((project, domain)) = route(request.url()) else {
        return (404, "endpoint absent\n".into());
    };
    let fail_config = || {
        (
            500,
            format!("project {project}: cannot read webhook configuration or signing reference\n"),
        )
    };
    let Ok(records) = PublishedRecord::load_all(home) else {
        return fail_config();
    };
    let Some(record) = records.iter().find(|record| record.project == project) else {
        return (404, "endpoint absent\n".into());
    };
    if !record.checkout.join("instrukt.yaml").is_file() {
        return fail_config();
    }
    let inspection = home.for_inspection();
    let Ok(config) = Configuration::load(&inspection, Some(&record.checkout)) else {
        return fail_config();
    };
    if !config
        .project()
        .is_ok_and(|declared| declared.as_ref() == Some(&project))
    {
        return fail_config();
    }
    let sources = match config.webhooks() {
        Ok(sources) => sources,
        Err(_) => return fail_config(),
    };
    let Some(source) = sources.get(&domain) else {
        return (404, "endpoint absent\n".into());
    };
    match config.module_enabled(Module::Events) {
        Ok(true) => {}
        Ok(false) => return (404, "source requires modules.events enabled\n".into()),
        Err(_) => return fail_config(),
    }
    if request.method() != &Method::Post {
        return (405, "POST required; no handshake\n".into());
    }
    let secrets = match source.resolve(home) {
        Ok(secrets) => secrets,
        Err(error) => return (500, format!("project {project}: {error}\n")),
    };
    let Ok(protocol) = source.protocol() else {
        return fail_config();
    };
    let signature = header(request, source.signature_header());
    let mut bytes = Vec::new();
    if request.as_reader().read_to_end(&mut bytes).is_err() {
        return (400, "invalid body\n".into());
    }
    if !_protocol::verify(&bytes, signature.as_deref(), &secrets, protocol) {
        return (401, "signature required or invalid\n".into());
    }
    let parsed = _protocol::decode(
        source,
        &domain,
        &bytes,
        header(request, "Content-Type").as_deref(),
        header(request, "X-GitHub-Event").as_deref(),
    );
    let Ok((event_type, data)) = parsed else {
        return (400, "malformed verified body\n".into());
    };
    let delivery = if protocol == WebhookProtocol::Github {
        header(request, "X-GitHub-Delivery").filter(|v| !v.trim().is_empty())
    } else {
        None
    };
    let identity = delivery.unwrap_or_else(|| format!("{:x}", Sha256::digest(&bytes)));
    let claim = occurrence_id(&[
        &domain,
        protocol.as_str(),
        "project",
        project.as_str(),
        &identity,
        "0",
    ]);
    let event = (|| -> Result<EventEnvelope, crate::events::EnvelopeError> {
        let kind = source.delivery_kind(
            event_type
                .strip_prefix(&format!("{domain}."))
                .unwrap_or(&event_type),
        );
        let subject = if let Some(field) = source.subject_from() {
            let mut value = &serde_json::Value::Object(data.clone());
            for component in field.split('.') {
                value = value.get(component).ok_or_else(|| {
                    crate::events::EnvelopeError::Declaration(format!(
                        "missing subject field {field}"
                    ))
                })?;
            }
            let identity = match value {
                serde_json::Value::String(text) if !text.is_empty() => text.clone(),
                serde_json::Value::Number(number) => number.to_string(),
                _ => {
                    return Err(crate::events::EnvelopeError::Declaration(
                        "subject field must be a nonempty string or number".into(),
                    ));
                }
            };
            format!(
                "{domain}:project:{project}/{}",
                crate::keyed::encode(&identity)
            )
        } else {
            format!("{domain}:project:{project}")
        };
        let mut extensions = BTreeMap::from([("inbound_verify".into(), protocol.as_str().into())]);
        if kind == instrukt_core::event_type::DeliveryKind::State {
            extensions.insert("inbound_delivery".into(), "state".into());
        }
        Ok(EventEnvelope {
            id: claim.clone(),
            event_type,
            source: computer.into(),
            subject: EventRef::new(ReferenceKind::InboundSource, &subject)?,
            time: now.try_into()?,
            data,
            home: EventHome::Project(project),
            home_owner: EventHomeOwner::Unresolved,
            related: Vec::new(),
            causationid: None,
            extensions,
        })
    })();
    let event = match event {
        Ok(event) => event,
        Err(error) => {
            tracing::warn!(event="webhook_subject_invalid",error=%error);
            return (400, "invalid subject data\n".into());
        }
    };
    let result = (|| -> Result<(), crate::transport::TransportError> {
        let writing = home.for_writing();
        // Only a verified, valid delivery reaches the writing migration boundary.
        Configuration::load(&writing, Some(&record.checkout))?;
        let transport = ScopedTransport::from_config(&config, env, &RunnerState::new(&writing))?;
        transport.publish_claimed(&event, &claim, CLAIM_TTL)?;
        Ok(())
    })();
    match result {
        Ok(()) => (202, "accepted\n".into()),
        Err(error) => {
            tracing::warn!(event = "webhook_relay_failed", error = %error);
            (502, "cluster publication failed; retry delivery\n".into())
        }
    }
}
