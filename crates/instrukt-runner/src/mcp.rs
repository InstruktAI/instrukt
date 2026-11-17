//! Agents' access to declared MCP servers (`mcp-access`): which declarations
//! a caller sees, whose login a call carries, and the call itself over the
//! network.
//!
//! The rules choose whose login a call needs; one [`LoginSource`] obtains it.
//! The call path, the visibility rules and the removal of resolved values
//! from output never depend on where a login comes from.

mod _client;
mod _logins;

use std::collections::BTreeMap;

use instrukt_core::config::{ConfigError, Configuration, LayerKind, McpServer};
use instrukt_core::home::Home;
use instrukt_core::ids::{ProjectSlug, SessionRef};
use instrukt_core::state_db::{StateDbError, StateRead, rusqlite::OptionalExtension};
use serde_json::Value;

pub use _client::Tool;
pub use _logins::StoredLogins;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    State(#[from] StateDbError),
    #[error("no MCP server `{name}` for this caller; it sees: {seen}")]
    UnknownServer { name: String, seen: String },
    #[error(
        "--company {asked} names another company than this session works for ({own}); a session sees its own company's servers"
    )]
    OtherCompany { asked: String, own: String },
    #[error(
        "--company is the local operator's, outside any session; this session sees the servers of its own scope"
    )]
    CompanyFromSession,
    #[error(
        "the call is refused: speaker {speaker} has no `{credential}` login of their own in company {company}"
    )]
    MissingSpeakerLogin {
        speaker: String,
        credential: String,
        company: String,
    },
    #[error(
        "the call is refused: company {company} declares no login of its own for MCP server `{server}`, and nobody is asking"
    )]
    MissingCompanyLogin { server: String, company: String },
    #[error("MCP server `{server}` at {url} cannot be reached: {cause}")]
    Unreachable {
        server: String,
        url: String,
        cause: String,
    },
    #[error("MCP server `{server}` reports a failure: {cause}")]
    Server { server: String, cause: String },
}

/// Where a server a caller sees is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// The machine's global layer.
    Machine,
    /// A company home.
    Company(ProjectSlug),
}

/// A server a caller sees.
#[derive(Debug, Clone)]
pub struct Visible {
    pub server: McpServer,
    pub origin: Origin,
}

/// The caller of `instrukt mcp`, as the runtime's caller identity and the
/// runner's recorded assignment establish it.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    /// The calling session; `None` for the local operator.
    pub session: Option<SessionRef>,
    /// The company home the session's recorded assignment names.
    pub company: Option<ProjectSlug>,
}

/// The person whose addressed message started the calling session's
/// current turn, as the runner recorded them with that message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Speaker {
    /// Their identity on the connector.
    pub identity: Option<String>,
    /// Their name as the provider gave it.
    pub name: String,
    /// Their email address as the provider gave it.
    pub email: Option<String>,
}

impl std::fmt::Display for Speaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.email {
            Some(email) => write!(f, "{} <{email}>", self.name),
            None => write!(f, "{} (no email address recorded)", self.name),
        }
    }
}

/// Whose login a call needs, as the rules decide it.
#[derive(Debug, Clone, Copy)]
pub enum Need<'a> {
    /// The speaker's own login: the value of `credential` in their member
    /// entry of `company`.
    Speaker {
        company: &'a ProjectSlug,
        speaker: &'a Speaker,
        credential: &'a str,
    },
    /// The company's own login, referenced by `reference`.
    Company {
        company: &'a ProjectSlug,
        reference: &'a str,
    },
    /// The machine's own login, referenced by `reference`.
    Machine { reference: &'a str },
}

/// A login a call carries: its value, and how output names it in place of
/// that value.
#[derive(Clone)]
pub struct Login {
    pub value: String,
    pub label: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Where a call obtains the login the rules ask for.
pub trait LoginSource {
    /// The login `need` asks for, or the refusal naming what is missing.
    fn login(&self, need: Need<'_>) -> Result<Login, McpError>;
}

/// What a caller sees: the scope, its servers by name and, for a session
/// answering a person, that person.
#[derive(Debug, Clone)]
pub struct View {
    /// The company whose servers the caller sees; `None` for the machine.
    pub company: Option<ProjectSlug>,
    pub servers: BTreeMap<ProjectSlug, Visible>,
    pub speaker: Option<Speaker>,
}

/// What `caller`, with `--company` given as `asked`, sees.
pub fn view(home: &Home, caller: &Caller, asked: Option<&ProjectSlug>) -> Result<View, McpError> {
    let company = match (&caller.session, &caller.company, asked) {
        (Some(_), Some(own), Some(asked)) if asked != own => {
            return Err(McpError::OtherCompany {
                asked: asked.to_string(),
                own: own.to_string(),
            });
        }
        (Some(_), Some(own), _) => Some(own.clone()),
        (Some(_), None, Some(_)) => return Err(McpError::CompanyFromSession),
        (Some(_), None, None) | (None, _, None) => None,
        (None, _, Some(asked)) => Some(asked.clone()),
    };
    let machine = Configuration::load_global(home)?.mcp_servers(LayerKind::Global)?;
    let Some(company) = company else {
        return Ok(View {
            company: None,
            servers: machine
                .into_iter()
                .map(|(name, server)| {
                    (
                        name,
                        Visible {
                            server,
                            origin: Origin::Machine,
                        },
                    )
                })
                .collect(),
            speaker: None,
        });
    };
    let own = Configuration::load_company(home, &company)?.mcp_servers(LayerKind::Company)?;
    let mut servers: BTreeMap<ProjectSlug, Visible> = own
        .into_iter()
        .map(|(name, server)| {
            (
                name,
                Visible {
                    server,
                    origin: Origin::Company(company.clone()),
                },
            )
        })
        .collect();
    // A machine-wide server that carries a login is offered to a company's
    // work only when the company declares it itself, which replaces it.
    for (name, server) in machine {
        if server.is_open() && !servers.contains_key(&name) {
            servers.insert(
                name,
                Visible {
                    server,
                    origin: Origin::Machine,
                },
            );
        }
    }
    let speaker = match (&caller.session, &caller.company) {
        (Some(session), Some(_)) => speaker(home, session)?,
        _ => None,
    };
    Ok(View {
        company: Some(company),
        servers,
        speaker,
    })
}

/// The speaker of `session`'s current turn: the sender recorded with the
/// addressed message of its conversation the runtime accepted and whose turn
/// has not finished, or whose send was begun and not yet recorded as
/// accepted. A conversation has at most one such message, since the next is
/// delivered only once the current turn has finished. A session bound to no
/// conversation has none.
pub fn speaker(home: &Home, session: &SessionRef) -> Result<Option<Speaker>, McpError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let row = db
        .conn()
        .query_row(
            "SELECT m.sender, m.sender_name, m.sender_email
             FROM connected_conversations c
             JOIN conversation_messages m
               ON m.company=c.company AND m.connector=c.connector AND m.conversation=c.conversation
             WHERE c.session=?1
               AND (m.state='sent' OR (m.state='pending' AND m.attempted_at IS NOT NULL))
             ORDER BY m.written_us, m.message LIMIT 1",
            [session.to_string()],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(StateDbError::sqlite(db.path()))?;
    Ok(row.map(|(identity, name, email)| Speaker {
        name: name
            .or_else(|| identity.clone())
            .unwrap_or_else(|| "an unnamed sender".to_owned()),
        identity,
        email: email.filter(|email| !email.trim().is_empty()),
    }))
}

impl View {
    /// The server named `name`, or the refusal naming those this caller
    /// sees.
    pub fn server(&self, name: &str) -> Result<&Visible, McpError> {
        ProjectSlug::parse(name)
            .ok()
            .and_then(|slug| self.servers.get(&slug))
            .ok_or_else(|| McpError::UnknownServer {
                name: name.to_owned(),
                seen: if self.servers.is_empty() {
                    "none".to_owned()
                } else {
                    self.servers
                        .keys()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                },
            })
    }

    /// The login a call to `visible` carries, obtained from `source`;
    /// `None` for an open server.
    pub fn login(
        &self,
        visible: &Visible,
        source: &dyn LoginSource,
    ) -> Result<Option<Login>, McpError> {
        let server = &visible.server;
        let need = match (&visible.origin, &server.credential, &self.speaker) {
            (Origin::Company(company), Some(credential), Some(speaker)) => Need::Speaker {
                company,
                speaker,
                credential,
            },
            (Origin::Company(company), Some(_), None) => match &server.login {
                Some(reference) => Need::Company { company, reference },
                None => {
                    return Err(McpError::MissingCompanyLogin {
                        server: server.name.to_string(),
                        company: company.to_string(),
                    });
                }
            },
            // A call that has a speaker never carries the machine's login;
            // a speaker is only ever seen in a company's view, which offers
            // no machine-wide server that carries one.
            (Origin::Machine, None, None) => match &server.login {
                Some(reference) => Need::Machine { reference },
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        source.login(need).map(Some)
    }
}

/// A connection target: the server's name for messages, its address and
/// the login its requests carry.
pub struct Target<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub authorization: Option<&'a str>,
}

/// The server's tool catalog.
pub fn list_tools(target: &Target<'_>) -> Result<Vec<Tool>, McpError> {
    _client::Session::open(target)?.list_tools()
}

/// The result of calling `tool` with `arguments`.
pub fn call(target: &Target<'_>, tool: &str, arguments: Value) -> Result<Value, McpError> {
    _client::Session::open(target)?.call(tool, arguments)
}

/// Removes every resolved login value from what a server returned before
/// it is printed or logged, naming the login in its place.
#[derive(Debug, Default)]
pub struct Scrub {
    logins: Vec<Login>,
}

impl Scrub {
    pub fn new(logins: impl IntoIterator<Item = Login>) -> Self {
        Self {
            logins: logins
                .into_iter()
                .filter(|login| !login.value.is_empty())
                .collect(),
        }
    }

    /// `text` with every login value replaced by its label.
    pub fn text(&self, text: &str) -> String {
        self.logins.iter().fold(text.to_owned(), |text, login| {
            text.replace(&login.value, &login.label)
        })
    }

    /// `value` with every login value in its strings and keys replaced.
    pub fn json(&self, value: Value) -> Value {
        match value {
            Value::String(text) => Value::String(self.text(&text)),
            Value::Array(items) => Value::Array(items.into_iter().map(|v| self.json(v)).collect()),
            Value::Object(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (self.text(&key), self.json(value)))
                    .collect(),
            ),
            other => other,
        }
    }
}
