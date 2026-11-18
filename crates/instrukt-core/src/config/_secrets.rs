//! Protected, literal secret storage: the machine's file and each company
//! home's own. No environment fallback.
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::PathBuf;
use std::time::Duration;

use super::ConfigError;
use crate::{
    fsx::{LockGuard, atomic_write_with_permissions},
    home::Home,
    ids::ProjectSlug,
};

pub(super) fn reference(text: &str) -> Option<&str> {
    let name = text.strip_prefix("${")?.strip_suffix('}')?;
    valid_name(name).then_some(name)
}

fn valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The protected file a `${NAME}` reference resolves from: the machine's
/// one secrets file, or a company home's own. Neither falls back to the
/// other, to the process environment or to any `.env`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretSource {
    /// `~/.instrukt/secrets.env`.
    Machine,
    /// `~/.instrukt/companies/<name>/secrets.env`.
    Company(ProjectSlug),
}

impl SecretSource {
    fn path(&self, home: &Home) -> PathBuf {
        match self {
            SecretSource::Machine => home.secrets_file(),
            SecretSource::Company(name) => home.company_secrets_file(name),
        }
    }

    /// The home whose file it is, as diagnostics name it.
    fn label(&self) -> String {
        match self {
            SecretSource::Machine => "machine-local secrets.env".into(),
            SecretSource::Company(name) => format!("company {name} secrets.env"),
        }
    }

    fn lock(&self) -> String {
        match self {
            SecretSource::Machine => "secrets.lock".into(),
            SecretSource::Company(name) => format!("secrets-company-{name}.lock"),
        }
    }
}

fn error(source: &SecretSource, reference: &str, cause: &str) -> ConfigError {
    ConfigError::InvalidValue {
        key: reference.into(),
        message: format!("{}: {cause}", source.label()),
    }
}

fn read(
    home: &Home,
    source: &SecretSource,
    name: &str,
) -> Result<BTreeMap<String, String>, ConfigError> {
    read_values(home, source).map_err(|cause| error(source, name, cause))
}

/// The values of `source`'s file, or why it cannot be read.
fn read_values(
    home: &Home,
    source: &SecretSource,
) -> Result<BTreeMap<String, String>, &'static str> {
    let path = source.path(home);
    let link = std::fs::symlink_metadata(&path).map_err(|_| "missing or unreadable file")?;
    if !link.file_type().is_file() {
        return Err("expected a regular owner-only file");
    }
    let mut file = std::fs::File::open(&path).map_err(|_| "unreadable file")?;
    let metadata = file.metadata().map_err(|_| "cannot inspect permissions")?;
    let owner = rustix::process::geteuid().as_raw();
    if metadata.uid() != owner || metadata.mode() & 0o077 != 0 || metadata.mode() & 0o400 == 0 {
        return Err("file must be readable by its owner only (chmod 600)");
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|_| "unreadable UTF-8 file")?;
    // Escape interpolation before dotenv parsing: stored material is literal.
    let literal = literal_values(&text);
    dotenvy::from_read_iter(literal.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| "malformed secret file")
}

/// One read of a secrets file, from which every reference resolves: what is
/// judged and what is used are then the same values.
pub(crate) struct SecretSnapshot {
    source: SecretSource,
    values: Result<BTreeMap<String, String>, &'static str>,
}

impl SecretSnapshot {
    pub(crate) fn read(home: &Home, source: &SecretSource) -> Self {
        Self {
            source: source.clone(),
            values: read_values(home, source),
        }
    }

    /// As [`resolve_secret_from`], from this read.
    pub(crate) fn resolve(&self, reference_text: &str) -> Result<String, ConfigError> {
        let name = reference(reference_text)
            .ok_or_else(|| error(&self.source, "secret reference", "expected exact ${NAME}"))?;
        let values = self
            .values
            .as_ref()
            .map_err(|cause| error(&self.source, reference_text, cause))?;
        values
            .get(name)
            .filter(|v| !v.trim().is_empty())
            .cloned()
            .ok_or_else(|| {
                error(
                    &self.source,
                    reference_text,
                    "missing or blank required value",
                )
            })
    }
}

/// Resolve a reference from the machine's protected secrets file.
pub fn resolve_secret(home: &Home, reference_text: &str) -> Result<String, ConfigError> {
    resolve_secret_from(home, &SecretSource::Machine, reference_text)
}

/// Resolve a reference from `source` alone.
pub fn resolve_secret_from(
    home: &Home,
    source: &SecretSource,
    reference_text: &str,
) -> Result<String, ConfigError> {
    let name = reference(reference_text)
        .ok_or_else(|| error(source, "secret reference", "expected exact ${NAME}"))?;
    read(home, source, reference_text)?
        .remove(name)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| error(source, reference_text, "missing or blank required value"))
}

/// Store one value in the machine's file without returning or logging
/// material.
pub fn set_secret(home: &Home, name: &str, value: &str) -> Result<(), ConfigError> {
    set_secret_in(home, &SecretSource::Machine, name, value)
}

/// Store one value in `source`'s file without returning or logging
/// material. This is the only writer used by the secrets command surface.
/// A company's file is written only inside its existing home.
pub fn set_secret_in(
    home: &Home,
    source: &SecretSource,
    name: &str,
    value: &str,
) -> Result<(), ConfigError> {
    if !valid_name(name) || value.trim().is_empty() || value.contains('\0') {
        return Err(error(
            source,
            "secret",
            "requires a valid name and nonblank value",
        ));
    }
    if let SecretSource::Company(company) = source {
        // Only an existing company home has a secrets file of its own.
        super::Configuration::load_company(&home.for_inspection(), company)?;
    }
    let machine = Home::at_user_home(home.machine_user_home().into());
    let _lock = LockGuard::acquire(
        &machine.locks_dir().join(source.lock()),
        Duration::from_secs(10),
    )?;
    let path = source.path(home);
    let mut values = if path
        .try_exists()
        .map_err(|_| error(source, name, "cannot inspect file"))?
    {
        read(home, source, name)?
    } else {
        BTreeMap::new()
    };
    values.insert(name.into(), value.into());
    let text: String = values
        .into_iter()
        .map(|(name, value)| {
            // dotenv double quotes support multiline material and escaped quotes.
            let escaped = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('$', "\\$");
            format!("{name}=\"{escaped}\"\n")
        })
        .collect();
    atomic_write_with_permissions(
        &path,
        text.as_bytes(),
        std::fs::Permissions::from_mode(0o600),
    )?;
    tracing::info!(event = "secret_written", reference = name, home = %source.label());
    Ok(())
}

/// Generate and store a random value in the machine's file.
pub fn generate_secret(home: &Home, name: &str) -> Result<(), ConfigError> {
    generate_secret_in(home, &SecretSource::Machine, name)
}

/// Generate and store a random value in `source`'s file.
pub fn generate_secret_in(
    home: &Home,
    source: &SecretSource,
    name: &str,
) -> Result<(), ConfigError> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| error(source, name, "secure random source unavailable"))?;
    set_secret_in(home, source, name, &hex::encode(bytes))
}

fn literal_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut prior_space = true;
    for ch in text.chars() {
        if comment {
            out.push(ch);
            if ch == '\n' {
                comment = false;
                prior_space = true;
            }
            continue;
        }
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' && quote != Some('\'') {
            out.push(ch);
            escaped = true;
        } else if ch == '#' && quote.is_none() && prior_space {
            comment = true;
            out.push(ch);
        } else if matches!(ch, '\'' | '"') && (quote.is_none() || quote == Some(ch)) {
            quote = if quote.is_some() { None } else { Some(ch) };
            out.push(ch);
        } else {
            if ch == '$' && quote != Some('\'') {
                out.push('\\');
            }
            out.push(ch);
        }
        prior_space = ch.is_whitespace();
    }
    out
}
