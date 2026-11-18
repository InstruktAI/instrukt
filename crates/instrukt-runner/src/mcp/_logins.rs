//! Logins stored as `${NAME}` references: a member's credential in their
//! company's settings, the company's or the machine's declared login, each
//! resolved from its own protected secrets file.

use instrukt_core::config::{Configuration, SecretSource, resolve_secret_from};
use instrukt_core::home::Home;

use super::{Login, LoginSource, McpError, Need};

/// The logins a home's settings reference.
pub struct StoredLogins<'a> {
    home: &'a Home,
}

impl<'a> StoredLogins<'a> {
    pub fn new(home: &'a Home) -> Self {
        Self { home }
    }

    fn resolve(&self, source: &SecretSource, reference: &str) -> Result<Login, McpError> {
        Ok(Login {
            value: resolve_secret_from(self.home, source, reference)?,
            label: reference.to_owned(),
        })
    }
}

impl LoginSource for StoredLogins<'_> {
    fn login(&self, need: Need<'_>) -> Result<Login, McpError> {
        match need {
            Need::Speaker {
                company,
                speaker,
                credential,
            } => {
                let members = Configuration::load_company(self.home, company)?.members()?;
                // The speaker's entry is the one member of this company whose
                // address equals theirs, without regard to case; names are
                // never compared.
                let reference = speaker
                    .email
                    .as_deref()
                    .and_then(|email| members.by_email(email))
                    .and_then(|member| member.credentials.get(credential))
                    .ok_or_else(|| McpError::MissingSpeakerLogin {
                        speaker: speaker.to_string(),
                        credential: credential.to_owned(),
                        company: company.to_string(),
                    })?;
                self.resolve(&SecretSource::Company(company.clone()), reference)
            }
            Need::Company { company, reference } => {
                self.resolve(&SecretSource::Company(company.clone()), reference)
            }
            Need::Machine { reference } => self.resolve(&SecretSource::Machine, reference),
        }
    }
}
