//! Real HTTP receiver, isolated project declarations and disposable Redis.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::panic,
    reason = "shared test setup"
)]
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use hmac::{Hmac, Mac};
use instrukt_runner::events::EventEnvelope;
use instrukt_runner::transport::EVENT_STREAM;
use sha2::Sha256;

use crate::common::redis::RedisServer;
use crate::support::Computer;

pub const SECRET: &str = "declaring-project-key";
pub const OLD: &str = "previous-project-key";

pub struct Receiver {
    child: Child,
    port: u16,
    logs: Option<JoinHandle<()>>,
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
        let mut child = computer
            .env
            .instrukt_process(&computer.env.path(""))
            .args(["webhooks", "serve"])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stderr = child.stderr.take().unwrap();
        let (ready, observed) = mpsc::channel();
        let logs = std::thread::spawn(move || {
            let mut startup = String::new();
            for line in BufReader::new(stderr).lines() {
                let line = line.unwrap();
                // This diagnostic only synchronizes startup. Each test's proof
                // is its real HTTP/Redis outcome, never a log assertion.
                if line.contains("webhooks_listening") && ready.send(Ok(())).is_err() {
                    return;
                }
                startup.push_str(&line);
                startup.push('\n');
            }
            if ready.send(Err(startup)).is_err() {
                tracing::debug!("receiver readiness observer already closed");
            }
        });
        let receiver = Self {
            child,
            port,
            logs: Some(logs),
        };
        match observed.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(())) => Ok(receiver),
            Ok(Err(detail)) => Err(detail),
            Err(error) => Err(format!("receiver readiness was not observed: {error}")),
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
        let header = if route.contains("/github/") {
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
        if let Some(logs) = self.logs.take() {
            logs.join().unwrap();
        }
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
    computer
        .project
        .write(".env", &format!("HOOK_KEY={SECRET}\nOLD_KEY={OLD}\n"));
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

pub const NATIVE: &str = "/hooks/inbound/project/fork";
pub const GITHUB: &str = "/hooks/inbound/github/project/fork";
pub const BODY: &[u8] = br#"{"domain":"erp","name":"invoice.paid","payload":{"invoice":42}}"#;
