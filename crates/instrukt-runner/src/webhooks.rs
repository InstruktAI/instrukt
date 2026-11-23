//! Signed, project- and company-scoped loopback intake. Acceptance ends at
//! cluster append.
mod _github;
mod _protocol;

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::Duration;

use instrukt_core::config::{
    Configuration, Env, Module, SecretSource, WebhookProtocol, company_homes, webhook_domain,
};
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
    #[error("no enabled registered project or company home declares a readable webhook source")]
    NoSources,
    #[error(transparent)]
    Registration(#[from] instrukt_core::publication::PublicationStateError),
    #[error("{home}: {detail}")]
    Declaration { home: String, detail: String },
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
                source
                    .resolve(home, &SecretSource::Machine)
                    .map_err(|error| error.to_string())?;
            }
            Ok(!sources.is_empty())
        })();
        match declaration {
            Ok(enabled) => demand |= enabled,
            Err(detail) => {
                tracing::warn!(event = "webhook_declaration_unreadable", project = %record.project, error = %detail);
                invalid = Some(WebhookError::Declaration {
                    home: format!("project {}", record.project),
                    detail,
                });
            }
        }
    }
    // A company home's sources add demand only while the machine's events
    // switch is on.
    if Configuration::load_global(&inspection)?.module_enabled(Module::Events)? {
        if ingress.url.is_some() {
            demand = true;
        }
        for (name, config) in company_homes(&inspection)? {
            let declaration = (|| -> Result<bool, String> {
                let config = config.map_err(|error| error.to_string())?;
                let sources = config.webhooks().map_err(|error| error.to_string())?;
                for source in sources.values() {
                    source
                        .resolve(home, &SecretSource::Company(name.clone()))
                        .map_err(|error| error.to_string())?;
                }
                Ok(!sources.is_empty())
            })();
            match declaration {
                Ok(enabled) => demand |= enabled,
                Err(detail) => {
                    tracing::warn!(event = "webhook_declaration_unreadable", company = %name, error = %detail);
                    invalid = Some(WebhookError::Declaration {
                        home: format!("company {name}"),
                        detail,
                    });
                }
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
    // Rebuild derived public per-company OIDC issuer documents in cluster Redis
    // from authoritative `company_issuers` + current `ingress.url` without rotating keys.
    let mut last_ingress_url = ingress.url.clone();
    if last_ingress_url.is_some()
        && let Err(error) = crate::mcp::republish_company_issuers(&inspection, None)
    {
        tracing::warn!(
            event = "issuer_republish_failed",
            error = %error,
            "could not republish public company issuer metadata at receiver startup"
        );
    }
    loop {
        // A valid placement change withdraws this receiver even when no requests arrive.
        match Configuration::load_global(&inspection).and_then(|config| config.ingress()) {
            Ok(current) if current.computer != computer => return Err(WebhookError::Placement),
            Ok(current) => {
                if current.url != last_ingress_url {
                    last_ingress_url = current.url;
                    if last_ingress_url.is_some()
                        && let Err(error) = crate::mcp::republish_company_issuers(&inspection, None)
                    {
                        tracing::warn!(
                            event = "issuer_republish_failed",
                            error = %error,
                            "could not republish public company issuer metadata after ingress.url change"
                        );
                    }
                }
            }
            Err(error) => tracing::warn!(event = "webhook_configuration_invalid", error = %error),
        }
        let Some(mut request) = server
            .recv_timeout(Duration::from_secs(1))
            .map_err(WebhookError::Listen)?
        else {
            continue;
        };
        let reply = if request.method() == &Method::Get
            && request.url() == crate::installation::_launch::PROBE_PATH
        {
            match crate::installation::_launch::running(address.port()) {
                Ok(proof) => HttpReply::plain(200, proof),
                Err(error) => {
                    tracing::warn!(event = "receiver_execution_unproven", error = %error);
                    HttpReply::plain(503, "receiver execution unproven\n".into())
                }
            }
        } else {
            receive_http(home, env, computer.as_str(), &mut request, clock())
        };
        let mut http_response = Response::from_string(reply.body).with_status_code(reply.status);
        for (name, value) in reply.headers {
            if let Ok(header) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
                http_response = http_response.with_header(header);
            }
        }
        if let Err(error) = request.respond(http_response) {
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
    if installed_port.is_some() {
        crate::installation::_launch::arm(home, address.port())
            .map_err(|e| WebhookError::Listen(std::io::Error::other(e)))?;
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

/// The kind of home a source path names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HomeKind {
    Project,
    Company,
}

impl HomeKind {
    fn as_str(self) -> &'static str {
        match self {
            HomeKind::Project => "project",
            HomeKind::Company => "company",
        }
    }
}

/// `/hooks/inbound/<kind>/<slug>/<domain>` for a project or a company home.
fn route(url: &str) -> Option<(HomeKind, ProjectSlug, String)> {
    let rest = url.strip_prefix("/hooks/inbound/")?;
    let (kind, rest) = rest.split_once('/')?;
    let kind = match kind {
        "project" => HomeKind::Project,
        "company" => HomeKind::Company,
        _ => return None,
    };
    let (slug, domain) = rest.split_once('/')?;
    if !webhook_domain(domain) {
        return None;
    }
    Some((kind, ProjectSlug::parse(slug).ok()?, domain.to_owned()))
}

/// The declaring home of one request, as read for it.
struct Declaring {
    kind: HomeKind,
    slug: ProjectSlug,
    /// The configuration holding the home's `webhooks:` declarations.
    sources: Configuration,
    /// The configuration supplying the events switch and cluster transport.
    transport: Configuration,
    secrets: SecretSource,
    /// The checkout of a project home, re-read on the writing path.
    checkout: Option<std::path::PathBuf>,
}

impl Declaring {
    fn label(&self) -> String {
        format!("{} {}", self.kind.as_str(), self.slug)
    }

    fn origin(&self) -> EventHome {
        match self.kind {
            HomeKind::Project => EventHome::Project(self.slug.clone()),
            HomeKind::Company => EventHome::Company(self.slug.clone()),
        }
    }

    fn owner(&self) -> EventHomeOwner {
        match self.kind {
            HomeKind::Project => EventHomeOwner::Unresolved,
            HomeKind::Company => EventHomeOwner::None,
        }
    }

    /// Persist a pending conversion of the declaring home's documents
    /// before publication; only a verified, valid delivery reaches it.
    fn migrate(&self, writing: &Home) -> Result<Configuration, instrukt_core::config::ConfigError> {
        match self.kind {
            HomeKind::Project => Configuration::load(writing, self.checkout.as_deref()),
            HomeKind::Company => {
                Configuration::load_company(writing, &self.slug)?;
                Configuration::load_global(writing)
            }
        }
    }
}

/// The declaring home of `slug`: `Err` carries the response for a home
/// that has no endpoint (404) or cannot be read (500).
fn declaring(home: &Home, kind: HomeKind, slug: ProjectSlug) -> Result<Declaring, (u16, String)> {
    let absent = || (404, "endpoint absent\n".to_owned());
    let unreadable = || {
        (
            500,
            format!(
                "{} {slug}: cannot read webhook configuration or signing reference\n",
                kind.as_str()
            ),
        )
    };
    let inspection = home.for_inspection();
    match kind {
        HomeKind::Project => {
            let records = PublishedRecord::load_all(home).map_err(|_| unreadable())?;
            let record = records
                .iter()
                .find(|record| record.project == slug)
                .ok_or_else(absent)?;
            if !record.checkout.join("instrukt.yaml").is_file() {
                return Err(unreadable());
            }
            let config = Configuration::load(&inspection, Some(&record.checkout))
                .map_err(|_| unreadable())?;
            if !config
                .project()
                .is_ok_and(|declared| declared.as_ref() == Some(&slug))
            {
                return Err(unreadable());
            }
            Ok(Declaring {
                kind,
                transport: config.clone(),
                sources: config,
                secrets: SecretSource::Machine,
                checkout: Some(record.checkout.clone()),
                slug,
            })
        }
        HomeKind::Company => {
            // A company without a home, or a folder whose document names
            // another company, is no company: it has no endpoint.
            let document = home.company_dir(&slug).join("instrukt.yaml");
            match std::fs::metadata(&document) {
                Ok(metadata) if metadata.is_file() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Err(absent());
                }
                // Anything else is an existing home this computer cannot
                // inspect: a failure, never absence.
                Ok(_) | Err(_) => return Err(unreadable()),
            }
            let sources = match Configuration::load_company(&inspection, &slug) {
                Ok(config) => config,
                Err(instrukt_core::config::ConfigError::NotACompany { .. }) => {
                    return Err(absent());
                }
                Err(_) => return Err(unreadable()),
            };
            let transport = Configuration::load_global(&inspection).map_err(|_| unreadable())?;
            Ok(Declaring {
                kind,
                sources,
                transport,
                secrets: SecretSource::Company(slug.clone()),
                checkout: None,
                slug,
            })
        }
    }
}

fn header(request: &Request, name: &str) -> Option<String> {
    let mut headers = request
        .headers()
        .iter()
        .filter(|header| header.field.as_str().as_str().eq_ignore_ascii_case(name));
    let value = headers.next()?.value.as_str().to_owned();
    headers.next().is_none().then_some(value)
}

struct HttpReply {
    status: u16,
    body: String,
    headers: Vec<(&'static str, &'static str)>,
}

impl HttpReply {
    fn plain(status: u16, body: String) -> Self {
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    fn json(status: u16, body: String) -> Self {
        Self {
            status,
            body,
            headers: vec![("Content-Type", "application/json")],
        }
    }

    fn sign_in_page() -> Self {
        Self {
            status: 200,
            body: "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Sign-in return received</title></head><body><p>Sign-in return received. You can return to your conversation.</p></body></html>\n".into(),
            headers: vec![
                ("Content-Type", "text/html; charset=utf-8"),
                ("Cache-Control", "no-store"),
                ("Referrer-Policy", "no-referrer"),
            ],
        }
    }
}

const MAX_SIGN_IN_URL_BYTES: usize = 8 * 1024;

fn is_hex_128(text: &str) -> bool {
    text.len() == 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn receive_http(
    home: &Home,
    env: &Env,
    computer: &str,
    request: &mut Request,
    now: Timestamp,
) -> HttpReply {
    let url = request.url();
    if url.starts_with("/hooks/inbound/sign-in/") {
        let mut reply = receive_sign_in(home, request, now);
        // Every reply on the sign-in return path carries no-store & no-referrer,
        // including 4xx/5xx error replies, because the request URL may carry `code`/`state`.
        if !reply
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("Cache-Control"))
        {
            reply.headers.push(("Cache-Control", "no-store"));
        }
        if !reply
            .headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("Referrer-Policy"))
        {
            reply.headers.push(("Referrer-Policy", "no-referrer"));
        }
        return reply;
    }
    if url.starts_with("/hooks/inbound/issuers/") {
        return receive_issuer(home, request);
    }
    let (status, body) = receive(home, env, computer, request, now);
    HttpReply::plain(status, body)
}

/// Handle `GET /hooks/inbound/sign-in/<return_id>?state=...&code=...` (or `error=...`).
/// Never logs query parameters, state, code, or tokens.
fn receive_sign_in(home: &Home, request: &Request, now: Timestamp) -> HttpReply {
    if request.method() != &Method::Get {
        return HttpReply::plain(405, "GET required\n".into());
    }
    let raw_url = request.url();
    if raw_url.len() > MAX_SIGN_IN_URL_BYTES {
        return HttpReply::plain(400, "request too large\n".into());
    }
    let (path, query) = match raw_url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (raw_url, None),
    };
    let Some(return_id) = path.strip_prefix("/hooks/inbound/sign-in/") else {
        return HttpReply::plain(404, "endpoint absent\n".into());
    };
    if !is_hex_128(return_id) {
        return HttpReply::plain(404, "endpoint absent\n".into());
    }
    let Some(query) = query else {
        // Return without `state` is discarded cleanly, showing the same plain page.
        return HttpReply::sign_in_page();
    };
    if !valid_percent_encoding(query) {
        return HttpReply::plain(400, "malformed query encoding\n".into());
    }

    let mut state: Option<String> = None;
    let mut code: Option<String> = None;
    let mut error: Option<String> = None;
    let mut iss: Option<String> = None;
    for (k, v) in form_urlencoded::parse(query.as_bytes()) {
        match k.as_ref() {
            "state" => {
                if state.is_some() || v.is_empty() {
                    return HttpReply::plain(400, "invalid sign-in return parameters\n".into());
                }
                state = Some(v.into_owned());
            }
            "code" => {
                if code.is_some() || v.is_empty() {
                    return HttpReply::plain(400, "invalid sign-in return parameters\n".into());
                }
                code = Some(v.into_owned());
            }
            "error" => {
                if error.is_some() || v.is_empty() {
                    return HttpReply::plain(400, "invalid sign-in return parameters\n".into());
                }
                error = Some(v.into_owned());
            }
            "iss" => {
                if iss.is_some() || v.is_empty() {
                    return HttpReply::plain(400, "invalid sign-in return parameters\n".into());
                }
                iss = Some(v.into_owned());
            }
            _ => {}
        }
    }

    let Some(state) = state else {
        return HttpReply::sign_in_page();
    };
    if code.is_some() == error.is_some() {
        return HttpReply::plain(
            400,
            "sign-in return requires exactly one of code or error\n".into(),
        );
    }

    let state_digest = hex::encode(Sha256::digest(state.as_bytes()));
    let entry = crate::mcp::SignInReturnEntry {
        return_id: return_id.to_owned(),
        code,
        error,
        iss,
    };

    let inspection = home.for_inspection();
    let global = match Configuration::load_global(&inspection) {
        Ok(config) => config,
        Err(_) => return HttpReply::plain(500, "cannot read ingress configuration\n".into()),
    };
    let transport = match ScopedTransport::cluster_only(&global, &RunnerState::new(&inspection)) {
        Ok(transport) => transport,
        Err(_) => return HttpReply::plain(502, "cluster Redis unavailable\n".into()),
    };
    match crate::mcp::store_sign_in_return(&transport, &state_digest, &entry, now.as_second()) {
        Ok(crate::mcp::StoreOutcome::Stored | crate::mcp::StoreOutcome::Duplicate) => {
            HttpReply::sign_in_page()
        }
        Ok(crate::mcp::StoreOutcome::Full) => {
            HttpReply::plain(503, "sign-in capacity reached; retry shortly\n".into())
        }
        Err(_) => HttpReply::plain(502, "cluster Redis unavailable\n".into()),
    }
}

fn valid_percent_encoding(query: &str) -> bool {
    let bytes = query.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    true
}

/// Handle read-only built-in per-company OIDC issuer endpoints:
/// - `GET /hooks/inbound/issuers/<issuer_id>/.well-known/openid-configuration`
/// - `GET /hooks/inbound/issuers/<issuer_id>/jwks.json`
fn receive_issuer(home: &Home, request: &Request) -> HttpReply {
    if request.method() != &Method::Get {
        return HttpReply::plain(405, "GET required\n".into());
    }
    let path = request.url();
    if path.contains('?') {
        return HttpReply::plain(404, "endpoint absent\n".into());
    }
    let Some(rest) = path.strip_prefix("/hooks/inbound/issuers/") else {
        return HttpReply::plain(404, "endpoint absent\n".into());
    };
    let Some((issuer_id, suffix)) = rest.split_once('/') else {
        return HttpReply::plain(404, "endpoint absent\n".into());
    };
    if !is_hex_128(issuer_id) {
        return HttpReply::plain(404, "endpoint absent\n".into());
    }
    let is_discovery = match suffix {
        ".well-known/openid-configuration" => true,
        "jwks.json" => false,
        _ => return HttpReply::plain(404, "endpoint absent\n".into()),
    };

    let inspection = home.for_inspection();
    let global = match Configuration::load_global(&inspection) {
        Ok(config) => config,
        Err(_) => return HttpReply::plain(500, "cannot read ingress configuration\n".into()),
    };
    let transport = match ScopedTransport::cluster_only(&global, &RunnerState::new(&inspection)) {
        Ok(transport) => transport,
        Err(_) => return HttpReply::plain(502, "cluster Redis unavailable\n".into()),
    };
    let doc = if is_discovery {
        crate::mcp::read_issuer_discovery(&transport, issuer_id)
    } else {
        crate::mcp::read_issuer_jwks(&transport, issuer_id)
    };
    match doc {
        Ok(Some(json_body)) => HttpReply::json(200, json_body),
        Ok(None) => HttpReply::plain(404, "issuer not found\n".into()),
        Err(_) => HttpReply::plain(502, "cluster Redis unavailable\n".into()),
    }
}

fn receive(
    home: &Home,
    env: &Env,
    computer: &str,
    request: &mut Request,
    now: Timestamp,
) -> (u16, String) {
    let Some((kind, slug, domain)) = route(request.url()) else {
        return (404, "endpoint absent\n".into());
    };
    let declaring = match declaring(home, kind, slug) {
        Ok(declaring) => declaring,
        Err(response) => return response,
    };
    let label = declaring.label();
    let fail_config = || {
        (
            500,
            format!("{label}: cannot read webhook configuration or signing reference\n"),
        )
    };
    let sources = match declaring.sources.webhooks() {
        Ok(sources) => sources,
        Err(_) => return fail_config(),
    };
    let Some(source) = sources.get(&domain) else {
        return (404, "endpoint absent\n".into());
    };
    match declaring.transport.module_enabled(Module::Events) {
        Ok(true) => {}
        Ok(false) => return (404, "source requires modules.events enabled\n".into()),
        Err(_) => return fail_config(),
    }
    if request.method() != &Method::Post {
        return (405, "POST required; no handshake\n".into());
    }
    let secrets = match source.resolve(home, &declaring.secrets) {
        Ok(secrets) => secrets,
        Err(error) => return (500, format!("{label}: {error}\n")),
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
        declaring.kind.as_str(),
        declaring.slug.as_str(),
        &identity,
        "0",
    ]);
    let qualified = format!("{domain}:{}:{}", declaring.kind.as_str(), declaring.slug);
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
            format!("{qualified}/{}", crate::keyed::encode(&identity))
        } else {
            qualified.clone()
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
            home: declaring.origin(),
            home_owner: declaring.owner(),
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
        declaring.migrate(&writing)?;
        let transport =
            ScopedTransport::from_config(&declaring.transport, env, &RunnerState::new(&writing))?;
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
