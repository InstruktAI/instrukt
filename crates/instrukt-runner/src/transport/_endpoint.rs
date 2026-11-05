//! Shared connection limits and reachability for one logical Redis database.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use instrukt_core::config::RedisEndpoint;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use redis::{ConnectionAddr, FromRedisValue, IntoConnectionInfo};

use super::TransportError;
use crate::state::{self, RunnerState, StateError};

pub(crate) struct Endpoint {
    pub identity: String,
    client: redis::Client,
    timeout: Duration,
    capacity: u64,
    pool: Mutex<Pool>,
    available: Condvar,
    state: RunnerState,
    reachability: Mutex<()>,
}

#[derive(Default)]
struct Pool {
    idle: Vec<redis::Connection>,
    open: u64,
}

impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Endpoint")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl Endpoint {
    pub fn new(
        settings: &RedisEndpoint,
        scope: &'static str,
        state: &RunnerState,
    ) -> Result<Self, TransportError> {
        let invalid = || TransportError::Url { scope };
        let mut info = settings
            .url
            .as_str()
            .into_connection_info()
            .map_err(|_| invalid())?;
        let mut redis = info.redis_settings().clone();
        if redis.username().is_some() || redis.password().is_some() {
            return Err(invalid());
        }
        if let Some(username) = &settings.username {
            redis = redis.set_username(username);
        }
        if let Some(password) = &settings.password {
            redis = redis.set_password(password);
        }
        let address = match info.addr() {
            ConnectionAddr::Unix(path) => {
                let absolute = if path.is_absolute() {
                    path.clone()
                } else {
                    return Err(invalid());
                };
                let resolved = absolute
                    .parent()
                    .and_then(|parent| parent.canonicalize().ok())
                    .zip(absolute.file_name())
                    .map_or_else(|| absolute.clone(), |(parent, name)| parent.join(name));
                format!("unix:{}", resolved.display())
            }
            ConnectionAddr::Tcp(host, port) | ConnectionAddr::TcpTls { host, port, .. } => {
                let host = if host.eq_ignore_ascii_case("localhost") {
                    "127.0.0.1".into()
                } else {
                    host.to_ascii_lowercase()
                };
                format!("tcp:{host}:{port}")
            }
            _ => return Err(invalid()),
        };
        let identity = crate::events::occurrence_id(&[&address, &redis.db().to_string()]);
        info = info.set_redis_settings(redis);
        let client = redis::Client::open(info).map_err(|_| invalid())?;
        Ok(Self {
            identity,
            client,
            timeout: Duration::from_secs(settings.socket_timeout),
            capacity: settings.max_connections,
            pool: Mutex::new(Pool::default()),
            available: Condvar::new(),
            state: state.clone(),
            reachability: Mutex::new(()),
        })
    }

    pub fn command<T: FromRedisValue>(
        &self,
        command: &mut redis::Cmd,
    ) -> Result<T, TransportError> {
        self.run(|conn| command.query(conn))
    }

    pub fn run<T>(
        &self,
        operation: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, TransportError> {
        let connection = {
            let mut pool = self
                .pool
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                if let Some(conn) = pool.idle.pop() {
                    break Ok(conn);
                }
                if pool.open < self.capacity {
                    pool.open += 1;
                    drop(pool);
                    break self.connect();
                }
                pool = self
                    .available
                    .wait(pool)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        };
        let (result, connection) = match connection {
            Ok(mut conn) => {
                let result = operation(&mut conn);
                (result, Some(conn))
            }
            Err(error) => (Err(error), None),
        };
        {
            let mut pool = self
                .pool
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if result.is_ok() {
                if let Some(conn) = connection {
                    pool.idle.push(conn);
                } else {
                    pool.open -= 1;
                }
            } else {
                pool.open -= 1;
            }
            self.available.notify_one();
        }
        // Database work begins only after the network operation and pool release.
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(redis::RedisError::is_io_error)
        {
            self.record_reachability(result.is_ok());
        }
        result.map_err(|error| TransportError::Redis {
            source_id: self.identity.clone(),
            kind: format!(
                "{:?}{}",
                error.kind(),
                error
                    .code()
                    .map_or(String::new(), |code| format!("/{code}"))
            ),
        })
    }

    fn connect(&self) -> redis::RedisResult<redis::Connection> {
        let conn = self.client.get_connection_with_timeout(self.timeout)?;
        conn.set_read_timeout(Some(self.timeout))?;
        conn.set_write_timeout(Some(self.timeout))?;
        Ok(conn)
    }

    fn record_reachability(&self, reachable: bool) {
        let _guard = self
            .reachability
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let result: Result<(), StateError> = self.state.write(|tx| {
            let path = state::db_path(tx);
            let previous: Option<bool> = tx.query_row("SELECT reachable FROM redis_reachability WHERE source = ?1", [&self.identity], |row| row.get(0)).optional().map_err(state::sql(&path))?;
            if previous == Some(reachable) { return Ok(()); }
            tx.execute("INSERT INTO redis_reachability (source, reachable) VALUES (?1, ?2) ON CONFLICT(source) DO UPDATE SET reachable = excluded.reachable", params![self.identity, reachable]).map_err(state::sql(&path))?;
            if reachable { tracing::info!(event = "redis_reachable", source = %self.identity); }
            else { tracing::warn!(event = "redis_unreachable", source = %self.identity); }
            Ok(())
        });
        if let Err(error) = result {
            tracing::warn!(event = "redis_reachability_not_recorded", error = %error);
        }
    }
}
