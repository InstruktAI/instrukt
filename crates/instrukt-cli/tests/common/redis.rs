//! Real, disposable Redis. Port zero disables TCP; only this test owns the socket.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tempfile::TempDir;

pub struct RedisServer {
    pub url: String,
    child: Child,
    dir: TempDir,
    home: PathBuf,
}

impl RedisServer {
    pub fn new(env: &super::TestEnv) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("instrukt-redis-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket: PathBuf = dir.path().join("redis.sock");
        let mut child = spawn(env.home(), dir.path(), &socket);
        let url = format!("unix://{}", socket.display());
        ready(&mut child, &url);
        Self {
            url,
            child,
            dir,
            home: env.home().to_path_buf(),
        }
    }

    pub fn restart(&mut self) {
        assert!(
            self.child.try_wait().unwrap().is_some(),
            "stop this disposable instance before restarting it"
        );
        let socket = Path::new(self.url.strip_prefix("unix://").unwrap());
        self.child = spawn(&self.home, self.dir.path(), socket);
        ready(&mut self.child, &self.url);
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

fn spawn(home: &Path, dir: &Path, socket: &Path) -> Child {
    Command::new("redis-server")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
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
        .arg(socket)
        .arg("--dir")
        .arg(dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn ready(child: &mut Child, url: &str) {
    let client = redis::Client::open(url).unwrap();
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
}
