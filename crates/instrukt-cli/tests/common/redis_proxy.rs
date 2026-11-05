//! A Redis network-boundary gate: withhold a delivered entry's response while
//! killing instrukt. Redis and SQLite remain real; production has no crash seam.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

pub struct DeliveryGate {
    pub url: String,
    state: Arc<Gate>,
    sockets: Arc<Mutex<Vec<UnixStream>>>,
    thread: Option<JoinHandle<()>>,
    _dir: tempfile::TempDir,
}

#[derive(Default)]
struct Gate {
    armed: AtomicBool,
    intercepted: AtomicBool,
    stopping: AtomicBool,
    released: Mutex<bool>,
    wake: Condvar,
}

impl DeliveryGate {
    pub fn new(server: &super::redis::RedisServer) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("instrukt-gate-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("redis.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let target = server.url.strip_prefix("unix://").unwrap().to_owned();
        let state = Arc::new(Gate::default());
        let sockets = Arc::new(Mutex::new(Vec::new()));
        let worker_state = Arc::clone(&state);
        let worker_sockets = Arc::clone(&sockets);
        let thread = std::thread::spawn(move || {
            let mut connections = Vec::new();
            while !worker_state.stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((front, _)) => {
                        if worker_state.stopping.load(Ordering::Acquire) {
                            break;
                        }
                        front.set_nonblocking(false).unwrap();
                        worker_sockets
                            .lock()
                            .unwrap()
                            .push(front.try_clone().unwrap());
                        let target = target.clone();
                        let state = Arc::clone(&worker_state);
                        connections.push(std::thread::spawn(move || {
                            let mut front = BufReader::new(front);
                            let mut back = BufReader::new(UnixStream::connect(target).unwrap());
                            while let Ok(command) = frame(&mut front) {
                                if back.get_mut().write_all(&command).is_err() {
                                    break;
                                }
                                let Ok(response) = frame(&mut back) else {
                                    break;
                                };
                                let arguments = redis::parse_redis_value(&command)
                                    .ok()
                                    .and_then(|value| {
                                        redis::from_redis_value::<Vec<Vec<u8>>>(value).ok()
                                    })
                                    .unwrap_or_default();
                                let has_delivery = redis::parse_redis_value(&response)
                                    .ok()
                                    .and_then(|value| {
                                        redis::from_redis_value::<redis::streams::StreamReadReply>(
                                            value,
                                        )
                                        .ok()
                                    })
                                    .is_some_and(|reply| {
                                        reply.keys.iter().any(|stream| !stream.ids.is_empty())
                                    });
                                if arguments
                                    .first()
                                    .is_some_and(|name| name.eq_ignore_ascii_case(b"XREADGROUP"))
                                    && has_delivery
                                    && state.armed.swap(false, Ordering::AcqRel)
                                {
                                    state.intercepted.store(true, Ordering::Release);
                                    let guard = state.released.lock().unwrap();
                                    drop(
                                        state
                                            .wake
                                            .wait_while(guard, |released| {
                                                !*released
                                                    && !state.stopping.load(Ordering::Acquire)
                                            })
                                            .unwrap(),
                                    );
                                }
                                if front.get_mut().write_all(&response).is_err() {
                                    break;
                                }
                            }
                        }));
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::yield_now()
                    }
                    Err(error) => panic!("delivery gate accept failed: {error}"),
                }
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            url: format!("unix://{}", socket.display()),
            state,
            sockets,
            thread: Some(thread),
            _dir: dir,
        }
    }

    pub fn arm(&self) {
        self.state.armed.store(true, Ordering::Release);
    }
    pub fn intercepted(&self) -> bool {
        self.state.intercepted.load(Ordering::Acquire)
    }
    pub fn release(&self) {
        *self.state.released.lock().unwrap() = true;
        self.state.wake.notify_all();
    }
}

impl Drop for DeliveryGate {
    fn drop(&mut self) {
        self.state.stopping.store(true, Ordering::Release);
        self.release();
        // Wake a blocking accept; a closed listener means it already stopped.
        let _wake_accept = UnixStream::connect(self.url.strip_prefix("unix://").unwrap());
        for stream in self.sockets.lock().unwrap().iter() {
            let _closed = stream.shutdown(std::net::Shutdown::Both);
        }
        self.thread.take().unwrap().join().unwrap();
    }
}

fn frame(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.read_until(b'\n', &mut bytes)?;
    if bytes.is_empty() {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    let count = || -> std::io::Result<i64> {
        std::str::from_utf8(&bytes[1..bytes.len().saturating_sub(2)])
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| std::io::ErrorKind::InvalidData.into())
    };
    match bytes[0] {
        b'$' => {
            let size = count()?;
            if size >= 0 {
                let mut value = vec![0; size as usize + 2];
                reader.read_exact(&mut value)?;
                bytes.extend(value);
            }
        }
        b'*' => {
            for _ in 0..count()?.max(0) {
                bytes.extend(frame(reader)?);
            }
        }
        b'+' | b'-' | b':' => {}
        _ => return Err(std::io::ErrorKind::InvalidData.into()),
    }
    Ok(bytes)
}
