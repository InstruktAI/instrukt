//! Protected, literal machine-local secret storage. No environment fallback.
use std::collections::BTreeMap;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::time::Duration;

use super::ConfigError;
use crate::{
    fsx::{LockGuard, atomic_write_with_permissions},
    home::Home,
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

fn error(reference: &str, cause: &str) -> ConfigError {
    ConfigError::InvalidValue {
        key: reference.into(),
        message: format!("machine-local secrets.env: {cause}"),
    }
}

fn read(home: &Home, name: &str) -> Result<BTreeMap<String, String>, ConfigError> {
    let path = home.secrets_file();
    let link =
        std::fs::symlink_metadata(&path).map_err(|_| error(name, "missing or unreadable file"))?;
    if !link.file_type().is_file() {
        return Err(error(name, "expected a regular owner-only file"));
    }
    let mut file = std::fs::File::open(&path).map_err(|_| error(name, "unreadable file"))?;
    let metadata = file
        .metadata()
        .map_err(|_| error(name, "cannot inspect permissions"))?;
    let owner = rustix::process::geteuid().as_raw();
    if metadata.uid() != owner || metadata.mode() & 0o077 != 0 || metadata.mode() & 0o400 == 0 {
        return Err(error(
            name,
            "file must be readable by its owner only (chmod 600)",
        ));
    }
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|_| error(name, "unreadable UTF-8 file"))?;
    // Escape interpolation before dotenv parsing: stored material is literal.
    let literal = literal_values(&text);
    dotenvy::from_read_iter(literal.as_bytes())
        .collect::<Result<_, _>>()
        .map_err(|_| error(name, "malformed secret file"))
}

pub fn resolve_secret(home: &Home, reference_text: &str) -> Result<String, ConfigError> {
    let name = reference(reference_text)
        .ok_or_else(|| error("secret reference", "expected exact ${NAME}"))?;
    read(home, reference_text)?
        .remove(name)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| error(reference_text, "missing or blank required value"))
}

/// Store one value without returning or logging material. This is the only
/// writer used by the secrets command surface.
pub fn set_secret(home: &Home, name: &str, value: &str) -> Result<(), ConfigError> {
    if !valid_name(name) || value.trim().is_empty() || value.contains('\0') {
        return Err(error("secret", "requires a valid name and nonblank value"));
    }
    let machine = Home::at_user_home(home.user_home().into());
    let _lock = LockGuard::acquire(
        &machine.locks_dir().join("secrets.lock"),
        Duration::from_secs(10),
    )?;
    let mut values = if home
        .secrets_file()
        .try_exists()
        .map_err(|_| error(name, "cannot inspect file"))?
    {
        read(home, name)?
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
        &home.secrets_file(),
        text.as_bytes(),
        std::fs::Permissions::from_mode(0o600),
    )?;
    Ok(())
}

pub fn generate_secret(home: &Home, name: &str) -> Result<(), ConfigError> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|_| error(name, "secure random source unavailable"))?;
    set_secret(home, name, &hex::encode(bytes))
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
