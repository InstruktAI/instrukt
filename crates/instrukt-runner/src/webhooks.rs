//! Signed, project-scoped loopback intake. Acceptance ends at cluster append.
mod _github;
mod _protocol;

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{Configuration, Env, Module, WebhookProtocol};
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

/// Internal service entry. Installation/socket inheritance is owned by S7.
pub fn serve(
    home: &Home,
    env: &Env,
    clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
) -> Result<(), WebhookError> {
    let computer = machine_id(home)?;
    let ingress = Configuration::load_global(home)?.ingress()?;
    if ingress.computer != computer {
        return Err(WebhookError::Placement);
    }
    let mut demand = false;
    let mut invalid = None;
    for record in PublishedRecord::load_all(home)? {
        let declaration = (|| -> Result<bool, String> {
            let config = Configuration::load(home, Some(&record.checkout))
                .map_err(|_| "cannot read webhook configuration".to_owned())?;
            let sources = config.webhooks().map_err(|error| error.to_string())?;
            if !config
                .module_enabled(Module::Events)
                .map_err(|_| "cannot read modules.events".to_owned())?
            {
                return Ok(false);
            }
            for source in sources.values() {
                source
                    .resolve(&record.checkout)
                    .map_err(|error| error.to_string())?;
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
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, ingress.port))
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
        match Configuration::load_global(home).and_then(|config| config.ingress()) {
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

fn route(url: &str) -> Option<(WebhookProtocol, ProjectSlug)> {
    let (protocol, slug) = if let Some(slug) = url.strip_prefix("/hooks/inbound/project/") {
        (WebhookProtocol::Native, slug)
    } else {
        (
            WebhookProtocol::Github,
            url.strip_prefix("/hooks/inbound/github/project/")?,
        )
    };
    Some((protocol, ProjectSlug::parse(slug).ok()?))
}

fn header(request: &Request, name: &'static str) -> Option<String> {
    let mut headers = request
        .headers()
        .iter()
        .filter(|header| header.field.equiv(name));
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
    let Some((protocol, project)) = route(request.url()) else {
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
    let Ok(config) = Configuration::load(home, Some(&record.checkout)) else {
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
    let Some(source) = sources.get(&protocol) else {
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
    let secrets = match source.resolve(&record.checkout) {
        Ok(secrets) => secrets,
        Err(error) => return (500, format!("project {project}: {error}\n")),
    };
    let signature_name = match protocol {
        WebhookProtocol::Native => "X-Hook-Signature",
        WebhookProtocol::Github => "X-Hub-Signature-256",
    };
    let signature = header(request, signature_name);
    let mut bytes = Vec::new();
    if request.as_reader().read_to_end(&mut bytes).is_err() {
        return (400, "invalid body\n".into());
    }
    if !_protocol::verify(&bytes, signature.as_deref(), &secrets) {
        return (401, "signature required or invalid\n".into());
    }
    let parsed = _protocol::decode(
        protocol,
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
        protocol.as_str(),
        "project",
        project.as_str(),
        &identity,
        "0",
    ]);
    let event = (|| -> Result<EventEnvelope, crate::events::EnvelopeError> {
        Ok(EventEnvelope {
            id: claim.clone(),
            event_type,
            source: computer.into(),
            subject: EventRef::new(
                ReferenceKind::InboundSource,
                &format!("{}:project:{project}", protocol.as_str()),
            )?,
            time: now.try_into()?,
            data,
            home: EventHome::Project(project),
            home_owner: EventHomeOwner::Unresolved,
            related: Vec::new(),
            causationid: None,
            extensions: BTreeMap::new(),
        })
    })();
    let result = (|| -> Result<(), crate::transport::TransportError> {
        let event = event?;
        let transport = ScopedTransport::from_config(&config, env, &RunnerState::new(home))?;
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
