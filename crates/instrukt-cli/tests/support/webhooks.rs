//! Real HTTP receiver, isolated project declarations and disposable Redis.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    reason = "shared test setup"
)]
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use hmac::{Hmac, Mac};
use instrukt_runner::events::EventEnvelope;
use instrukt_runner::transport::EVENT_STREAM;
use sha2::Sha256;

use crate::common::redis::RedisServer;
use crate::fixtures::event_transport::Computer;

pub const SECRET: &str = "declaring-project-key";
pub const OLD: &str = "previous-project-key";

pub struct Receiver {
    child: Child,
    port: u16,
}

impl Receiver {
    pub fn start(computer: &Computer, cluster: &RedisServer) -> Self {
        computer.config(cluster, None);
        computer.tick();
        let machine = computer.computer();
        let global = computer.env.instrukt_home().join("instrukt.yaml");
        let base = std::fs::read_to_string(&global).unwrap();
        for _ in 0..10 {
            // Keep the reservation through setup, then wait for this child's own
            // bind completion. A random connection can belong to another test.
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            std::fs::write(
                &global,
                format!("{base}ingress: {{computer: '{machine}', port: {port}}}\n"),
            )
            .unwrap();
            drop(listener);
            match Self::spawn(computer, port) {
                Ok(receiver) => return receiver,
                Err(detail) if detail.contains("Address already in use") => continue,
                Err(detail) => panic!("receiver did not start: {detail}"),
            }
        }
        panic!("could not reserve a loopback port after ten bind collisions");
    }

    fn spawn(computer: &Computer, port: u16) -> Result<Self, String> {
        let log = computer
            .env
            .home()
            .join(".local/state/instrukt-ai/instrukt/webhooks.log");
        let offset = match std::fs::metadata(&log) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.to_string()),
        };
        let child = computer
            .env
            .instrukt_process(&computer.env.path(""))
            .args(["webhooks", "serve"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut receiver = Self { child, port };
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if receiver.child.try_wait().unwrap().is_some() {
                let mut error = String::new();
                receiver
                    .child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut error)
                    .unwrap();
                return Err(error);
            }
            match std::fs::File::open(&log) {
                Ok(mut file) => {
                    file.seek(SeekFrom::Start(offset)).unwrap();
                    let mut startup = String::new();
                    file.read_to_string(&mut startup).unwrap();
                    // Only this spawn's new records synchronize readiness.
                    // The tests assert real HTTP/Redis outcomes, never logs.
                    if startup.contains("event=webhooks_listening") {
                        return Ok(receiver);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            if Instant::now() >= deadline {
                return Err("receiver readiness was not observed in its log".into());
            }
            std::thread::yield_now();
        }
    }

    pub fn request(
        &self,
        method: &str,
        route: &str,
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        write!(stream, "{method} {route} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: {}\r\n", body.len()).unwrap();
        for (name, value) in headers {
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        stream.write_all(b"\r\n").unwrap();
        stream.write_all(body).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        (
            head.split_whitespace().nth(1).unwrap().parse().unwrap(),
            body.into(),
        )
    }

    pub fn signed(
        &self,
        route: &str,
        body: &[u8],
        secret: &str,
        extra: &[(&str, &str)],
    ) -> (u16, String) {
        let signature = signature(body, secret);
        let header = if route.ends_with("/github") {
            "X-Hub-Signature-256"
        } else {
            "X-Hook-Signature"
        };
        let mut headers = vec![(header, signature.as_str())];
        headers.extend_from_slice(extra);
        self.request("POST", route, body, &headers)
    }

    pub fn exited(&mut self) -> bool {
        self.child.try_wait().unwrap().is_some()
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
        }
        self.child.wait().unwrap();
    }
}

pub fn signature(body: &[u8], secret: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

pub fn declare(computer: &Computer, declaration: &str) {
    computer.project.write(
        "instrukt.yaml",
        &format!("schema_version: 1\nproject: fork\nwebhooks: {declaration}\n"),
    );
    for (name, value) in [("HOOK_KEY", SECRET), ("OLD_KEY", OLD)] {
        computer
            .env
            .instrukt(computer.project.path())
            .args(["secrets", "set", name])
            .write_stdin(value)
            .assert()
            .success();
    }
}

pub fn events(redis: &RedisServer) -> Vec<EventEnvelope> {
    let reply: redis::streams::StreamRangeReply = redis::cmd("XRANGE")
        .arg(EVENT_STREAM)
        .arg("-")
        .arg("+")
        .query(&mut redis.connection())
        .unwrap();
    reply
        .ids
        .iter()
        .map(|entry| EventEnvelope::decode(&entry.get::<String>("event").unwrap()).unwrap())
        .collect()
}

pub const NATIVE: &str = "/hooks/inbound/project/fork/erp";
pub const GITHUB: &str = "/hooks/inbound/project/fork/github";
pub const BODY: &[u8] = br#"{"name":"invoice.paid","payload":{"invoice":42}}"#;
