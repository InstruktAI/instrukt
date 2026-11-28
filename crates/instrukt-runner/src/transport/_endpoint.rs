//! Shared connection limits and reachability for one logical Redis database.

use std::sync::{Condvar, Mutex};
use std::time::Duration;

use instrukt_core::config::RedisEndpoint;
use instrukt_core::home::Home;
use redis::{ConnectionAddr, FromRedisValue, IntoConnectionInfo};

use super::{Reachability, RedisFailure, TransportError};
use crate::state::RunnerState;

pub(super) fn endpoint_identity_from_url(
    url: &str,
    scope: &'static str,
) -> Result<String, TransportError> {
    let invalid = || TransportError::Url { scope };
    let info = url.into_connection_info().map_err(|_| invalid())?;
    let redis = info.redis_settings();
    if redis.username().is_some() || redis.password().is_some() {
        return Err(invalid());
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
    Ok(crate::events::occurrence_id(&[
        &address,
        &redis.db().to_string(),
    ]))
}

pub(crate) struct Endpoint {
    pub identity: String,
    client: redis::Client,
    timeout: Duration,
    capacity: u64,
    pool: Mutex<Pool>,
    available: Condvar,
    home: Home,
    reachability: Mutex<()>,
    /// Whether this endpoint keeps its source's reachability record.
    records: Reachability,
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
        let identity = endpoint_identity_from_url(&settings.url, scope)?;
        let mut info = settings
            .url
            .as_str()
            .into_connection_info()
            .map_err(|_| invalid())?;
        let mut redis = info.redis_settings().clone();
        if let Some(username) = &settings.username {
            redis = redis.set_username(username);
        }
        if let Some(password) = &settings.password {
            redis = redis.set_password(password);
        }
        info = info.set_redis_settings(redis);
        let client = redis::Client::open(info).map_err(|_| invalid())?;
        Ok(Self {
            identity,
            client,
            timeout: Duration::from_secs(settings.socket_timeout),
            capacity: settings.max_connections,
            pool: Mutex::new(Pool::default()),
            available: Condvar::new(),
            home: state.home().clone(),
            reachability: Mutex::new(()),
            records: Reachability::Recorded,
        })
    }

    /// This endpoint keeping, or not keeping, its source's reachability record
    /// in [`Home::redis_reachability_db_file`].
    pub fn with_reachability(mut self, records: Reachability) -> Self {
        self.records = records;
        self
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
        // Reachability recording begins only after the network operation and pool release.
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(redis::RedisError::is_io_error)
        {
            self.record_reachability(result.is_ok());
        }
        result.map_err(|error| self.redis_error(error))
    }

    /// One bounded attempt owns this short-lived connection. It never borrows
    /// a reader slot, even when local and cluster share this endpoint.
    pub fn run_best_effort<T>(
        &self,
        timeout: Duration,
        operation: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, TransportError> {
        let timeout = timeout.min(self.timeout);
        let result = (|| {
            let mut conn = self.client.get_connection_with_timeout(timeout)?;
            conn.set_read_timeout(Some(timeout))?;
            conn.set_write_timeout(Some(timeout))?;
            operation(&mut conn)
        })();
        result.map_err(|error| self.redis_error(error))
    }

    /// One operation that may block for up to `wait`, on a short-lived
    /// connection of its own whose read timeout is `wait` plus this
    /// endpoint's socket timeout, so the wait ends before the connection
    /// times out. Reachability is recorded as for [`Self::run`]; pooled
    /// connections and their timeouts are untouched.
    pub fn run_waiting<T>(
        &self,
        wait: Duration,
        operation: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
    ) -> Result<T, TransportError> {
        let result = (|| {
            let mut conn = self.client.get_connection_with_timeout(self.timeout)?;
            conn.set_read_timeout(Some(wait.saturating_add(self.timeout)))?;
            conn.set_write_timeout(Some(self.timeout))?;
            operation(&mut conn)
        })();
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(redis::RedisError::is_io_error)
        {
            self.record_reachability(result.is_ok());
        }
        result.map_err(|error| self.redis_error(error))
    }

    fn redis_error(&self, error: redis::RedisError) -> TransportError {
        let class = if error.is_io_error() || error.is_connection_refusal() {
            RedisFailure::Connection
        } else {
            RedisFailure::Reply
        };
        TransportError::Redis {
            source_id: self.identity.clone(),
            class,
            kind: format!(
                "{}{}",
                error.category(),
                error
                    .code()
                    .map_or(String::new(), |code| format!("/{code}"))
            ),
        }
    }

    fn connect(&self) -> redis::RedisResult<redis::Connection> {
        let conn = self.client.get_connection_with_timeout(self.timeout)?;
        conn.set_read_timeout(Some(self.timeout))?;
        conn.set_write_timeout(Some(self.timeout))?;
        Ok(conn)
    }

    fn record_reachability(&self, reachable: bool) {
        if self.records == Reachability::Unrecorded {
            return;
        }
        super::_reachability::record(&self.home, &self.identity, reachable, &self.reachability);
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, Endpoint, RedisEndpoint, RunnerState};
    use instrukt_core::home::Home;
    use std::net::TcpListener;
    use std::sync::mpsc;

    #[test]
    fn an_in_flight_announcement_leaves_the_only_reader_slot_available() {
        // Disable Redis handshake commands. The listener keeps both TCP
        // connections open while the closures signal when they get a slot.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let url = format!("redis://{}/", listener.local_addr().unwrap());
        let state = RunnerState::new(&Home::at_user_home(std::path::PathBuf::from("/dev/null")));
        let mut endpoint = Endpoint::new(
            &RedisEndpoint {
                url,
                username: None,
                password: None,
                max_connections: 1,
                socket_timeout: 5,
            },
            "cluster",
            &state,
        )
        .unwrap();
        let info = endpoint.client.get_connection_info().clone();
        let settings = info.redis_settings().clone().set_skip_set_lib_name();
        endpoint.client = redis::Client::open(info.set_redis_settings(settings)).unwrap();
        let (writing, started) = mpsc::channel();
        let (release, held) = mpsc::channel();
        let (reading, proceeded) = mpsc::channel();
        std::thread::scope(|threads| {
            let endpoint = &endpoint;
            threads.spawn(move || {
                endpoint
                    .run_best_effort(Duration::from_secs(1), |_| {
                        writing.send(()).unwrap();
                        held.recv().unwrap();
                        Ok(())
                    })
                    .unwrap();
            });
            started.recv_timeout(Duration::from_secs(2)).unwrap();
            threads.spawn(move || {
                // A Redis command refusal does not write reachability state.
                endpoint
                    .run::<()>(|_| {
                        reading.send(()).unwrap();
                        Err(redis::RedisError::from((
                            redis::ErrorKind::Client,
                            "test command",
                        )))
                    })
                    .unwrap_err();
            });
            let during_announcement = proceeded.recv_timeout(Duration::from_secs(2));
            release.send(()).unwrap();
            assert!(
                during_announcement.is_ok(),
                "reader waited for the announcement socket"
            );
        });
    }
}
