//! Real, disposable Redis. Port zero disables TCP; only this test owns the socket.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct RedisServer {
    pub url: String,
    child: Child,
    _dir: TempDir,
}

impl RedisServer {
    pub fn new(env: &super::TestEnv) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("instrukt-redis-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket: PathBuf = dir.path().join("redis.sock");
        let mut child = Command::new("redis-server")
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", env.home())
            .args([
                "--port",
                "0",
                "--save",
                "",
                "--appendonly",
                "no",
                "--unixsocketperm",
                "700",
                "--unixsocket",
            ])
            .arg(&socket)
            .arg("--dir")
            .arg(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let url = format!("unix://{}", socket.display());
        let client = redis::Client::open(url.as_str()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(mut conn) = client.get_connection()
                && redis::cmd("PING").query::<String>(&mut conn).is_ok()
            {
                break;
            }
            assert!(
                child.try_wait().unwrap().is_none(),
                "disposable Redis stopped during startup"
            );
            assert!(
                Instant::now() < deadline,
                "disposable Redis failed to start"
            );
            std::thread::yield_now();
        }
        Self {
            url,
            child,
            _dir: dir,
        }
    }

    pub fn connection(&self) -> redis::Connection {
        redis::Client::open(self.url.as_str())
            .unwrap()
            .get_connection()
            .unwrap()
    }

    pub fn stop(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for RedisServer {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
        }
        self.child.wait().unwrap();
    }
}
