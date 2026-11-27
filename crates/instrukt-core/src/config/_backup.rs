//! Global home backup configuration (`backup` in `~/.instrukt/instrukt.yaml`)
//! (`project/instrukt/spec/feature/configuration/layered-configuration`,
//! `project/instrukt/spec/feature/runner/home-backup`).

use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;

use serde_json::json;
use serde_yaml_ng::Value;
use zeroize::Zeroizing;

use super::_error::{ConfigError, Finding, FindingKind};
use super::_interpolate::{join, key_text};
use super::_layer::{Layer, inspected, read};
use super::_schema::{KeyFinding, LayerKind, non_blank_pattern, secret_reference};
use super::_secrets::{SecretSnapshot, SecretSource, reference};
use crate::home::Home;

/// The top-level global-layer key holding home backup configuration.
pub const BACKUP_KEY: &str = "backup";

/// Default generation retention count when `backup.keep` is omitted.
pub const DEFAULT_BACKUP_KEEP: u32 = 14;

/// Default local daily schedule time (`HH:MM`, 24-hour) when `backup.time` is omitted.
pub const DEFAULT_BACKUP_TIME: &str = "03:00";

const REQUIRED_FIELDS: [&str; 6] = [
    "endpoint",
    "region",
    "bucket",
    "access_key_id",
    "secret_access_key",
    "age_recipient",
];

const OPTIONAL_FIELDS: [&str; 3] = ["prefix", "keep", "time"];

/// Daily local 24-hour schedule time (`HH:MM`) for home backup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackupTime {
    hour: u8,
    minute: u8,
}

impl BackupTime {
    /// Parses an exact 24-hour `HH:MM` clock time (`00:00`–`23:59`).
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        let bytes = text.as_bytes();
        if bytes.len() != 5
            || bytes[2] != b':'
            || !bytes[0].is_ascii_digit()
            || !bytes[1].is_ascii_digit()
            || !bytes[3].is_ascii_digit()
            || !bytes[4].is_ascii_digit()
        {
            return Err("expected a 24-hour local time in HH:MM form (00:00 to 23:59)");
        }
        let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
        let minute = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
        if hour > 23 || minute > 59 {
            return Err("expected a 24-hour local time in HH:MM form (00:00 to 23:59)");
        }
        Ok(Self { hour, minute })
    }

    pub fn hour(self) -> u8 {
        self.hour
    }

    pub fn minute(self) -> u8 {
        self.minute
    }
}

impl Default for BackupTime {
    fn default() -> Self {
        Self { hour: 3, minute: 0 }
    }
}

impl fmt::Display for BackupTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}

/// A resolved secret value held in memory for I/O only: redacted in `Debug`,
/// zeroized on drop, and never serializable.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Validated, admitted global home backup configuration with resolved
/// credentials held solely for storage I/O. Never implements `Serialize`.
#[derive(Clone, PartialEq, Eq)]
pub struct BackupConfig {
    pub endpoint: url::Url,
    pub region: String,
    pub bucket: String,
    pub access_key_id: SecretValue,
    pub secret_access_key: SecretValue,
    pub age_recipient: age::x25519::Recipient,
    pub prefix: Option<String>,
    pub keep: NonZeroU32,
    pub time: BackupTime,
}

impl fmt::Debug for BackupConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackupConfig")
            .field("endpoint", &self.endpoint.as_str())
            .field("region", &self.region)
            .field("bucket", &self.bucket)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &self.secret_access_key)
            .field("age_recipient", &self.age_recipient.to_string())
            .field("prefix", &self.prefix)
            .field("keep", &self.keep)
            .field("time", &self.time)
            .finish()
    }
}

/// Three-state admission result for the machine's global `backup` declaration.
#[derive(Debug, Clone)]
pub enum BackupAdmission {
    /// Global layer has no `backup` declaration (or global layer does not exist).
    Unconfigured,
    /// Global `backup` declaration validated and resolved its protected credentials.
    Admitted(Box<BackupConfig>),
    /// Global `backup` declaration is present but invalid or missing required secrets.
    Withheld(Vec<Finding>),
}

/// Whether `finding` withholds home backup admission while leaving the rest of
/// the global layer admitted.
pub fn withholds_backup(finding: &Finding) -> bool {
    finding.key == BACKUP_KEY || finding.key.starts_with("backup.")
}

/// Inspects the global layer's `backup` declaration directly without loading
/// `agents.yaml` or published projects.
pub fn inspect_global_backup(home: &Home) -> Result<BackupAdmission, ConfigError> {
    let path = home.global_config();
    let Some(doc) = read(&path)? else {
        return Ok(BackupAdmission::Unconfigured);
    };
    let global = inspected(home, &Layer::Global, &path, doc.value)?;
    let Some(raw) = global.get(BACKUP_KEY).filter(|value| !value.is_null()) else {
        return Ok(BackupAdmission::Unconfigured);
    };
    let secrets = SecretSnapshot::read(home, &SecretSource::Machine);
    let mut key_findings = Vec::new();
    check(raw, BACKUP_KEY, &mut key_findings);
    check_secrets(raw, &secrets, &mut key_findings);
    if !key_findings.is_empty() {
        let findings = key_findings
            .into_iter()
            .map(|finding| Finding {
                document: path.clone(),
                key: finding.key,
                kind: finding.kind,
                message: finding.message,
            })
            .collect();
        return Ok(BackupAdmission::Withheld(findings));
    }
    let config = parse_with_secrets(raw, &secrets)?;
    Ok(BackupAdmission::Admitted(Box::new(config)))
}

/// Resolves `backup` from an effective configuration's value using `<selected INSTRUKT_HOME>/secrets.env`.
pub(super) fn from_effective(
    home: &Home,
    value: &Value,
) -> Result<Option<BackupConfig>, ConfigError> {
    let Some(raw) = value.get(BACKUP_KEY).filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let secrets = SecretSnapshot::read(home, &SecretSource::Machine);
    let mut findings = Vec::new();
    check(raw, BACKUP_KEY, &mut findings);
    check_secrets(raw, &secrets, &mut findings);
    if let Some(first) = findings.into_iter().next() {
        return Err(ConfigError::InvalidValue {
            key: first.key,
            message: first.message,
        });
    }
    parse_with_secrets(raw, &secrets).map(Some)
}

/// Redacts non-reference literal credentials under `backup` before display or interpolation.
pub(super) fn redact(value: &mut Value) {
    let Some(backup) = value.get_mut(BACKUP_KEY).and_then(Value::as_mapping_mut) else {
        return;
    };
    for field in ["access_key_id", "secret_access_key"] {
        if let Some(entry) = backup.get_mut(field)
            && !entry.as_str().is_some_and(super::is_secret_reference)
        {
            *entry = Value::String("<protected-file reference>".into());
        }
    }
}

/// Validates the shape of `backup` at `path`.
pub(super) fn check(value: &Value, path: &str, out: &mut Vec<KeyFinding>) {
    let Some(map) = value.as_mapping() else {
        out.push(KeyFinding::new(
            path,
            FindingKind::InvalidValue,
            "expected a mapping",
        ));
        return;
    };
    for required in REQUIRED_FIELDS {
        if !map.get(required).is_some_and(|v| !v.is_null()) {
            out.push(KeyFinding::new(
                &join(path, required),
                FindingKind::Missing,
                "missing",
            ));
        }
    }
    for (key, field_value) in map {
        let name = key_text(key);
        let child = join(path, &name);
        if !REQUIRED_FIELDS.contains(&name.as_str()) && !OPTIONAL_FIELDS.contains(&name.as_str()) {
            out.push(KeyFinding::new(
                &child,
                FindingKind::UnknownKey,
                "unknown key",
            ));
            continue;
        }
        if field_value.is_null() {
            continue;
        }
        match name.as_str() {
            "endpoint" => {
                if !field_value
                    .as_str()
                    .is_some_and(super::_webhooks::ingress_url_admitted)
                {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected an absolute https URL (or loopback http URL) without embedded credentials, query or fragment",
                    ));
                }
            }
            "region" => {
                if !field_value.as_str().is_some_and(|s| !s.trim().is_empty()) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a non-empty region name",
                    ));
                }
            }
            "bucket" => {
                if !field_value.as_str().is_some_and(valid_bucket) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a non-empty bucket name without slashes or whitespace",
                    ));
                }
            }
            "access_key_id" | "secret_access_key" => {
                if !field_value.as_str().is_some_and(|s| reference(s).is_some()) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "credentials must be an exact `${NAME}` secret reference into `~/.instrukt/secrets.env`; store the value with `instrukt secrets set <NAME>`",
                    ));
                }
            }
            "age_recipient" => {
                let valid = field_value
                    .as_str()
                    .is_some_and(|s| age::x25519::Recipient::from_str(s.trim()).is_ok());
                if !valid {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a valid public age recipient (`age1...`), never a private key",
                    ));
                }
            }
            "prefix" => {
                if !field_value.as_str().is_some_and(valid_prefix) {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a relative key prefix with no leading/trailing `/`, empty segments, or `.`/`..` traversal segments",
                    ));
                }
            }
            "keep" => {
                if field_value
                    .as_u64()
                    .is_none_or(|n| n == 0 || u32::try_from(n).is_err())
                {
                    out.push(KeyFinding::new(
                        &child,
                        FindingKind::InvalidValue,
                        "expected a positive integer generation retention count",
                    ));
                }
            }
            "time" => match field_value.as_str() {
                Some(text) => {
                    if let Err(reason) = BackupTime::parse(text) {
                        out.push(KeyFinding::new(&child, FindingKind::InvalidValue, reason));
                    }
                }
                None => out.push(KeyFinding::new(
                    &child,
                    FindingKind::InvalidValue,
                    "expected a 24-hour local time string in HH:MM form (00:00 to 23:59)",
                )),
            },
            _ => {}
        }
    }
}

/// Checks that `backup.access_key_id` and `backup.secret_access_key` secret
/// references resolve to non-blank values in `<selected INSTRUKT_HOME>/secrets.env`.
pub(super) fn check_secrets(value: &Value, secrets: &SecretSnapshot, out: &mut Vec<KeyFinding>) {
    let Some(map) = value.as_mapping() else {
        return;
    };
    for field in ["access_key_id", "secret_access_key"] {
        let Some(ref_text) = map.get(field).and_then(Value::as_str) else {
            continue;
        };
        let Some(var_name) = reference(ref_text) else {
            continue;
        };
        if let Err(error) = secrets.resolve(ref_text) {
            out.push(KeyFinding::new(
                &join(BACKUP_KEY, field),
                FindingKind::Missing,
                format!("{error}; store it with `instrukt secrets set {var_name}`"),
            ));
        }
    }
}

/// JSON Schema describing `backup` in the global layer.
pub(super) fn json_schema() -> serde_json::Value {
    let non_blank = non_blank_pattern();
    json!({
        "type": "object",
        "description": "Encrypted nightly offsite backup settings for this computer's instrukt home (global layer only).",
        "required": REQUIRED_FIELDS,
        "additionalProperties": false,
        "properties": {
            "endpoint": {
                "type": "string",
                "format": "uri",
                "description": "Provider-neutral S3-compatible endpoint URL (`https://...`, or loopback `http://...` for local verification) with no embedded credentials, query, or fragment."
            },
            "region": {
                "type": "string",
                "pattern": non_blank,
                "description": "Non-empty S3 region name."
            },
            "bucket": {
                "type": "string",
                "pattern": non_blank,
                "description": "Non-empty target S3 bucket name."
            },
            "access_key_id": secret_reference(
                "S3 access key ID for home backup",
                LayerKind::Global,
            ),
            "secret_access_key": secret_reference(
                "S3 secret access key for home backup",
                LayerKind::Global,
            ),
            "age_recipient": {
                "type": "string",
                "pattern": "^age1[0-9a-z]+$",
                "description": "Public age X25519 recipient (`age1...`) used for client-side encryption; keep the matching private identity offline outside the home."
            },
            "prefix": {
                "type": "string",
                "pattern": non_blank,
                "description": "Optional object key prefix under which this computer's machine-ID namespace is stored (no leading/trailing `/` or `.`/`..` segments)."
            },
            "keep": {
                "type": "integer",
                "minimum": 1,
                "default": DEFAULT_BACKUP_KEEP,
                "description": "Positive integer number of backup generations retained per machine."
            },
            "time": {
                "type": "string",
                "pattern": "^([01][0-9]|2[0-3]):[0-5][0-9]$",
                "default": DEFAULT_BACKUP_TIME,
                "description": "Daily local schedule time in 24-hour `HH:MM` form for the owned OS backup timer."
            }
        }
    })
}

fn valid_bucket(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && trimmed == text
        && !text.contains('/')
        && !text.chars().any(char::is_whitespace)
}

fn valid_prefix(text: &str) -> bool {
    if text.is_empty()
        || text.starts_with('/')
        || text.ends_with('/')
        || text.contains('\0')
        || text.contains('\\')
    {
        return false;
    }
    text.split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn parse_with_secrets(raw: &Value, secrets: &SecretSnapshot) -> Result<BackupConfig, ConfigError> {
    let map = raw.as_mapping().ok_or_else(|| ConfigError::InvalidValue {
        key: BACKUP_KEY.into(),
        message: "expected a mapping".into(),
    })?;
    let str_field = |name: &str| -> Result<&str, ConfigError> {
        map.get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| ConfigError::InvalidValue {
                key: join(BACKUP_KEY, name),
                message: "missing or invalid string".into(),
            })
    };
    let endpoint_raw = str_field("endpoint")?.trim();
    let endpoint = url::Url::parse(endpoint_raw).map_err(|err| ConfigError::InvalidValue {
        key: "backup.endpoint".into(),
        message: err.to_string(),
    })?;
    let region = str_field("region")?.trim().to_owned();
    let bucket = str_field("bucket")?.to_owned();
    let access_key_id = SecretValue::new(secrets.resolve(str_field("access_key_id")?)?);
    let secret_access_key = SecretValue::new(secrets.resolve(str_field("secret_access_key")?)?);
    let age_recipient = age::x25519::Recipient::from_str(str_field("age_recipient")?.trim())
        .map_err(|err| ConfigError::InvalidValue {
            key: "backup.age_recipient".into(),
            message: err.to_string(),
        })?;
    let prefix = map
        .get("prefix")
        .filter(|v| !v.is_null())
        .and_then(Value::as_str)
        .map(str::to_owned);
    let keep_u32 = match map.get("keep").filter(|v| !v.is_null()) {
        Some(v) => v
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| ConfigError::InvalidValue {
                key: "backup.keep".into(),
                message: "expected a positive integer".into(),
            })?,
        None => DEFAULT_BACKUP_KEEP,
    };
    let keep = NonZeroU32::new(keep_u32).ok_or_else(|| ConfigError::InvalidValue {
        key: "backup.keep".into(),
        message: "expected a positive integer".into(),
    })?;
    let time = match map.get("time").filter(|v| !v.is_null()) {
        Some(v) => {
            let text = v.as_str().ok_or_else(|| ConfigError::InvalidValue {
                key: "backup.time".into(),
                message: "expected HH:MM string".into(),
            })?;
            BackupTime::parse(text).map_err(|message| ConfigError::InvalidValue {
                key: "backup.time".into(),
                message: message.into(),
            })?
        }
        None => BackupTime::default(),
    };
    Ok(BackupConfig {
        endpoint,
        region,
        bucket,
        access_key_id,
        secret_access_key,
        age_recipient,
        prefix,
        keep,
        time,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Configuration, Env, GLOBAL_SCHEMA_VERSION, set_secret};

    fn test_recipient() -> String {
        age::x25519::Identity::generate().to_public().to_string()
    }

    fn valid_yaml(recipient: &str) -> String {
        [
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}"),
            "backup:".to_owned(),
            "  endpoint: https://s3.example.test".to_owned(),
            "  region: us-east-1".to_owned(),
            "  bucket: instrukt-backups".to_owned(),
            "  access_key_id: ${BACKUP_KEY_ID}".to_owned(),
            "  secret_access_key: ${BACKUP_SECRET}".to_owned(),
            format!("  age_recipient: {recipient}"),
            String::new(),
        ]
        .join("\n")
    }

    #[test]
    fn backup_time_validates_24_hour_clock_bounds() {
        assert_eq!(BackupTime::parse("00:00").unwrap().to_string(), "00:00");
        assert_eq!(BackupTime::parse("03:15").unwrap().hour(), 3);
        assert_eq!(BackupTime::parse("03:15").unwrap().minute(), 15);
        assert_eq!(BackupTime::parse("23:59").unwrap().to_string(), "23:59");
        for bad in ["3:00", "24:00", "12:60", "ab:cd", "03:00:00", ""] {
            assert!(BackupTime::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn inspect_global_backup_distinguishes_unconfigured_withheld_and_admitted() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        assert!(matches!(
            inspect_global_backup(&home).unwrap(),
            BackupAdmission::Unconfigured
        ));

        let recipient = test_recipient();
        crate::fsx::atomic_write(&home.global_config(), valid_yaml(&recipient).as_bytes()).unwrap();
        // Missing secrets withhold admission without failing.
        let withheld = inspect_global_backup(&home).unwrap();
        let BackupAdmission::Withheld(findings) = withheld else {
            panic!("expected withheld, got {withheld:?}");
        };
        assert!(findings.iter().any(|f| f.key == "backup.access_key_id"));
        assert!(findings.iter().any(|f| f.key == "backup.secret_access_key"));
        assert!(findings.iter().all(withholds_backup));

        set_secret(&home, "BACKUP_KEY_ID", "ak-test-value").unwrap();
        set_secret(&home, "BACKUP_SECRET", "sk-test-value").unwrap();
        let BackupAdmission::Admitted(cfg) = inspect_global_backup(&home).unwrap() else {
            panic!("expected admitted");
        };
        assert_eq!(cfg.keep.get(), DEFAULT_BACKUP_KEEP);
        assert_eq!(cfg.time.to_string(), DEFAULT_BACKUP_TIME);
        assert_eq!(cfg.access_key_id.expose(), "ak-test-value");
        assert_eq!(cfg.secret_access_key.expose(), "sk-test-value");
        // Debug never discloses secrets.
        let dbg = format!("{cfg:?}");
        assert!(!dbg.contains("ak-test-value"));
        assert!(!dbg.contains("sk-test-value"));
    }

    #[test]
    fn validation_rejects_bad_endpoint_prefix_recipient_and_literal_secrets() {
        let yaml: Value = serde_yaml_ng::from_str(
            "endpoint: http://user:pass@remote.example.test\n\
             region: ''\n\
             bucket: bad/bucket\n\
             access_key_id: AKIA_LITERAL\n\
             secret_access_key: secret_literal\n\
             age_recipient: AGE-SECRET-KEY-1QQQQQQQQQQQQQQ\n\
             prefix: ../escape\n\
             keep: 0\n\
             time: '25:99'\n\
             extra_unknown: true\n",
        )
        .unwrap();
        let mut findings = Vec::new();
        check(&yaml, BACKUP_KEY, &mut findings);
        let keys: Vec<&str> = findings.iter().map(|f| f.key.as_str()).collect();
        for expected in [
            "backup.endpoint",
            "backup.region",
            "backup.bucket",
            "backup.access_key_id",
            "backup.secret_access_key",
            "backup.age_recipient",
            "backup.prefix",
            "backup.keep",
            "backup.time",
            "backup.extra_unknown",
        ] {
            assert!(keys.contains(&expected), "missing {expected} in {keys:?}");
        }
        // None of the findings echo literal secret or private key text.
        for finding in &findings {
            assert!(!finding.message.contains("AKIA_LITERAL"));
            assert!(!finding.message.contains("secret_literal"));
            assert!(!finding.message.contains("AGE-SECRET-KEY-"));
        }
    }

    #[test]
    fn config_get_preserves_references_and_redacts_literal_backup_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf());
        let recipient = test_recipient();
        let yaml = [
            format!("schema_version: {GLOBAL_SCHEMA_VERSION}"),
            "backup:".to_owned(),
            "  endpoint: http://127.0.0.1:9000".to_owned(),
            "  region: local".to_owned(),
            "  bucket: test".to_owned(),
            "  access_key_id: ${BACKUP_KEY_ID}".to_owned(),
            "  secret_access_key: literal-secret-leaked".to_owned(),
            format!("  age_recipient: {recipient}"),
            String::new(),
        ]
        .join("\n");
        crate::fsx::atomic_write(&home.global_config(), yaml.as_bytes()).unwrap();
        let cfg = Configuration::load_global(&home).unwrap();
        let env = Env::from_pairs([("BACKUP_KEY_ID", "SHOULD_NOT_EXPAND")]);
        let view = cfg.get(Some("backup"), &env).unwrap();
        assert_eq!(view["access_key_id"].as_str(), Some("${BACKUP_KEY_ID}"));
        assert_eq!(
            view["secret_access_key"].as_str(),
            Some("<protected-file reference>")
        );
    }
}
