//! Machine-owned event transport settings; secrets are expanded only for transport.

use serde_yaml_ng::Value;

use super::_error::ConfigError;
use super::_interpolate::mask_references;
use super::{Configuration, Env};

pub const REDIS_BLOCK_SECONDS: u64 = 10;
pub const REDIS_MAX_CONNECTIONS: u64 = 50;
pub const REDIS_SOCKET_TIMEOUT: u64 = 60;

/// A single logical Redis database. Debug output deliberately excludes credentials.
#[derive(Clone)]
pub struct RedisEndpoint {
    pub url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub max_connections: u64,
    pub socket_timeout: u64,
}

impl std::fmt::Debug for RedisEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisEndpoint").finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct RedisSettings {
    pub cluster: RedisEndpoint,
    pub local: Option<RedisEndpoint>,
}

pub(super) fn settings(config: &Configuration, env: &Env) -> Result<RedisSettings, ConfigError> {
    let raw = config.redis_raw();
    if raw.is_some_and(|value| !value.is_mapping()) {
        return Err(ConfigError::InvalidValue {
            key: "redis".into(),
            message: "expected a mapping".into(),
        });
    }
    let endpoint = |scope: &str| -> Result<Option<RedisEndpoint>, ConfigError> {
        let invalid = |field: &str, message: &str| ConfigError::InvalidValue {
            key: if field.is_empty() {
                format!("redis.{scope}")
            } else {
                format!("redis.{scope}.{field}")
            },
            message: message.into(),
        };
        let Some(raw) = raw.and_then(|value| value.get(scope)) else {
            return Ok(None);
        };
        if !raw.is_mapping() {
            return Err(invalid("", "expected a mapping"));
        }
        let Some(url) = raw.get("url") else {
            return Ok(None);
        };
        let url = url
            .as_str()
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| invalid("url", "expected a nonempty Redis URL string"))?;
        let expanded = config.expand(url, env)?;
        if scope == "local" && !expanded.starts_with("unix://") {
            return Err(invalid("url", "local Redis requires a unix:// socket URL"));
        }
        let number = |key: &str, default| -> Result<u64, ConfigError> {
            match raw.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_u64()
                    .filter(|n| *n > 0)
                    .ok_or_else(|| invalid(key, "expected a positive integer")),
            }
        };
        let socket_timeout = number("socket_timeout", REDIS_SOCKET_TIMEOUT)?;
        if socket_timeout <= REDIS_BLOCK_SECONDS {
            return Err(invalid(
                "socket_timeout",
                "must outlast the 10 second blocking read",
            ));
        }
        let secret = |key: &str| -> Result<Option<String>, ConfigError> {
            let Some(value) = raw.get(key) else {
                return Ok(None);
            };
            let Some(text) = value.as_str().filter(|s| mask_references(s, "reference").1) else {
                return Err(invalid(key, "credentials must be references"));
            };
            let resolved = config.expand(text, env)?;
            if resolved.trim().is_empty() {
                return Err(invalid(key, "credential reference is empty"));
            }
            Ok(Some(resolved))
        };
        Ok(Some(RedisEndpoint {
            url: expanded,
            username: secret("username")?,
            password: secret("password")?,
            max_connections: number("max_connections", REDIS_MAX_CONNECTIONS)?,
            socket_timeout,
        }))
    };
    Ok(RedisSettings {
        cluster: endpoint("cluster")?.ok_or_else(|| ConfigError::InvalidValue {
            key: "redis.cluster.url".into(),
            message: "required for enabled event transport".into(),
        })?,
        local: endpoint("local")?,
    })
}

pub(super) fn redact(value: &mut Value, path: &str) {
    match value {
        Value::Mapping(map) => {
            for (key, value) in map {
                if let Some(key) = key.as_str() {
                    redact(
                        value,
                        &if path.is_empty() {
                            key.into()
                        } else {
                            format!("{path}.{key}")
                        },
                    );
                }
            }
        }
        Value::Sequence(seq) => {
            for item in seq {
                redact(item, path);
            }
        }
        _ if path.starts_with("redis.")
            && matches!(
                path.rsplit('.').next(),
                Some("url" | "username" | "password")
            ) =>
        {
            *value = Value::String("[redacted]".into());
        }
        _ => {}
    }
}
