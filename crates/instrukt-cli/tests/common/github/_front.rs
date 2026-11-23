//! The fake's public address: it accepts every connection for the
//! fake's whole life and relays each to the HTTP server while the fake
//! answers, ending connections when it is stopped.

use std::collections::BTreeMap;
use std::io;
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

use super::_model::State;
use super::_serve::serve;

/// What the fake's public address does with a connection.
#[derive(Debug)]
pub(super) enum Reach {
    /// Relays it to the HTTP server.
    Answering,
    /// Ends it before any byte of a response.
    Stopped,
    /// Still ends it, until the accept thread takes `marker`, the
    /// connection `resume` made, from the listen queue. The queue hands
    /// connections over in the order they were made (first in, first out
    /// on Linux and macOS), so every one taken before `marker` was made
    /// while the fake was stopped. Taking `marker` answers again and tells
    /// `resumed`.
    Resuming {
        marker: SocketAddr,
        resumed: Sender<()>,
    },
    /// The fake is being dropped: accepting ends.
    Ending,
    /// No thread accepts on it any more: accepting ended on a failure the
    /// fake recorded, and connections wait unanswered in the listen queue.
    Unattended,
}

/// The public address's transport state: its reach and both sides of every
/// connection it is relaying.
pub(super) struct Front {
    pub(super) reach: Reach,
    pub(super) next_connection: u64,
    pub(super) relayed: BTreeMap<u64, Relayed>,
    /// Failures of the fake's own transport, each with its operation and
    /// cause, reported to the test when the fake ends.
    pub(super) failures: Vec<String>,
}

/// One connection the fake is relaying.
pub(super) struct Relayed {
    pub(super) client: TcpStream,
    /// The relay's connection to the HTTP server once it is made, with the
    /// address the HTTP server sees its requests come from.
    pub(super) server: Option<(SocketAddr, TcpStream)>,
}

impl Relayed {
    /// Ends the connection on both sides, whatever exchange it is in.
    pub(super) fn close(self) {
        // A side its peer already closed leaves nothing to end.
        let _client_closed = self.client.shutdown(Shutdown::Both);
        if let Some((_, server)) = self.server {
            let _server_closed = server.shutdown(Shutdown::Both);
        }
    }
}

impl Front {
    /// Ends every relayed connection on both sides, whatever exchange it is
    /// in, and forgets it.
    pub(super) fn close_relayed(&mut self) {
        for relayed in std::mem::take(&mut self.relayed).into_values() {
            relayed.close();
        }
    }

    /// Ends relayed connection `id` on both sides and forgets it.
    pub(super) fn close(&mut self, id: u64) {
        if let Some(relayed) = self.relayed.remove(&id) {
            relayed.close();
        }
    }

    /// Whether the HTTP server's request from `peer` came over a connection
    /// the fake is still relaying.
    pub(super) fn relays_from(&self, peer: SocketAddr) -> bool {
        self.relayed.values().any(|relayed| {
            relayed
                .server
                .as_ref()
                .is_some_and(|(from, _)| *from == peer)
        })
    }

    /// Answers again and tells the waiting `resume`.
    pub(super) fn answer_again(&mut self) {
        if let Reach::Resuming { resumed, .. } =
            std::mem::replace(&mut self.reach, Reach::Answering)
        {
            // A `resume` that is no longer waiting has nothing to learn.
            let _told = resumed.send(());
        }
    }

    pub(super) fn fail(&mut self, operation: &str, cause: &dyn std::fmt::Display) {
        self.failures.push(format!("{operation}: {cause}"));
    }
}

/// The transport state, also after a fake thread panicked while holding
/// it: the address keeps its reach whatever failed a request.
pub(super) fn lock_front(front: &Mutex<Front>) -> MutexGuard<'_, Front> {
    front.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Accepts on the fake's public address until the fake is dropped, relaying
/// each connection to `backend` while the fake answers. However accepting
/// ends, the address is left `Unattended` unless the fake is being dropped.
pub(super) fn accept(
    listener: Arc<TcpListener>,
    backend: SocketAddr,
    front: Arc<Mutex<Front>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut relays: Vec<JoinHandle<()>> = Vec::new();
        run_part(&front, "accept on the public address", || {
            accept_connections(&listener, backend, &front, &mut relays);
        });
        let mut transport = lock_front(&front);
        if !matches!(transport.reach, Reach::Ending) {
            // Replacing a `Resuming` reach drops its `resumed`: the waiting
            // `resume` learns that accepting ended.
            transport.reach = Reach::Unattended;
        }
        drop(transport);
        // Each relay records its own panic.
        for relay in relays {
            let _ended = relay.join();
        }
    })
}

/// Takes connections from the public address until the fake is dropped or
/// accepting fails, which it records.
pub(super) fn accept_connections(
    listener: &TcpListener,
    backend: SocketAddr,
    front: &Arc<Mutex<Front>>,
    relays: &mut Vec<JoinHandle<()>>,
) {
    for connection in listener.incoming() {
        let client = match connection {
            Ok(client) => client,
            // A connection its client ended before it was accepted
            // leaves no one to answer.
            Err(cause) if peer_left_or_interrupted(&cause) => continue,
            Err(cause) => {
                lock_front(front).fail("accept on the public address", &cause);
                return;
            }
        };
        let mut transport = lock_front(front);
        match &transport.reach {
            // Only this thread leaves the address unattended, once it ends.
            Reach::Ending | Reach::Unattended => return,
            // Dropping the connection ends it before any response.
            Reach::Stopped => continue,
            Reach::Resuming { marker, .. } => {
                let marker = *marker;
                if client.peer_addr().ok() == Some(marker) {
                    transport.answer_again();
                }
                continue;
            }
            Reach::Answering => {}
        }
        let registered = match client.try_clone() {
            Ok(registered) => registered,
            Err(cause) => {
                transport.fail("hold a relayed connection", &cause);
                continue;
            }
        };
        let id = transport.next_connection;
        transport.next_connection += 1;
        transport.relayed.insert(
            id,
            Relayed {
                client: registered,
                server: None,
            },
        );
        drop(transport);
        // A finished relay recorded its own panic; its handle has nothing
        // left to tell.
        relays.retain(|relay| !relay.is_finished());
        match relay(client, backend, id, Arc::clone(front)) {
            Ok(relay) => relays.push(relay),
            Err(cause) => {
                let mut transport = lock_front(front);
                transport.fail("start a relay for a connection", &cause);
                transport.close(id);
            }
        }
    }
}

/// Runs `part` of one of the fake's threads. A panic in it is recorded as a
/// failure of `operation`, with its message, instead of ending the thread
/// unseen.
pub(super) fn run_part(front: &Mutex<Front>, operation: &str, part: impl FnOnce()) {
    if let Err(panic) = std::panic::catch_unwind(AssertUnwindSafe(part)) {
        let message = panic
            .downcast_ref::<&str>()
            .map(|message| (*message).to_owned())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "a panic without a message".to_owned());
        lock_front(front).fail(operation, &format!("the thread panicked: {message}"));
    }
}

/// Relays one client connection to the HTTP server until either side ends
/// it, then ends and forgets it, also after a panic in the relay.
pub(super) fn relay(
    client: TcpStream,
    backend: SocketAddr,
    id: u64,
    front: Arc<Mutex<Front>>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().spawn(move || {
        run_part(&front, "relay a connection", || {
            match TcpStream::connect(backend) {
                Ok(server) => {
                    if hold_server_side(&front, id, &server) {
                        pipe(client, server, &front);
                    }
                }
                Err(cause) => lock_front(&front).fail("connect to the HTTP server", &cause),
            }
        });
        lock_front(&front).close(id);
    })
}

/// Registers the relay's connection to the HTTP server with relayed
/// connection `id`, before any request crosses it. False when the fake
/// stopped since it accepted the connection, which closed it, or when the
/// server side cannot be held.
pub(super) fn hold_server_side(front: &Mutex<Front>, id: u64, server: &TcpStream) -> bool {
    let held = server
        .local_addr()
        .and_then(|from| Ok((from, server.try_clone()?)));
    let mut transport = lock_front(front);
    match held {
        Ok(held) => match transport.relayed.get_mut(&id) {
            Some(relayed) => {
                relayed.server = Some(held);
                true
            }
            None => false,
        },
        Err(cause) => {
            transport.fail("hold a relayed connection", &cause);
            false
        }
    }
}

/// Whether an accept failed only because the connection's client left
/// first or a signal interrupted the call.
pub(super) fn peer_left_or_interrupted(cause: &io::Error) -> bool {
    matches!(
        cause.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
    )
}

/// Copies bytes both ways between `client` and `server`; when the server
/// side ends, both connections end.
pub(super) fn pipe(mut client: TcpStream, mut server: TcpStream, front: &Arc<Mutex<Front>>) {
    let (mut request_from, mut request_to) = match (client.try_clone(), server.try_clone()) {
        (Ok(from), Ok(to)) => (from, to),
        (Err(cause), _) | (_, Err(cause)) => {
            lock_front(front).fail("relay a connection", &cause);
            return;
        }
    };
    let copying = Arc::clone(front);
    let upstream = std::thread::Builder::new().spawn(move || {
        run_part(&copying, "copy a relayed request", || {
            // Either side closing ends the copy; the half-close tells the
            // server no further request follows.
            let _copied = io::copy(&mut request_from, &mut request_to);
            let _closed = request_to.shutdown(Shutdown::Write);
        });
    });
    let upstream = match upstream {
        Ok(upstream) => upstream,
        // The relay ends both sides of the connection when this returns.
        Err(cause) => {
            lock_front(front).fail("start a relay's request copy", &cause);
            return;
        }
    };
    // Either side closing ends the copy, and ending both connections
    // releases the upstream copy.
    let _copied = io::copy(&mut server, &mut client);
    let _client_closed = client.shutdown(Shutdown::Both);
    let _server_closed = server.shutdown(Shutdown::Both);
    // The copy records its own panic.
    let _ended = upstream.join();
}

/// Serves the HTTP server's requests until it is unblocked.
pub(super) fn serve_requests(
    server: Arc<tiny_http::Server>,
    front: Arc<Mutex<Front>>,
    state: Arc<Mutex<State>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        run_part(&front, "serve the HTTP server's requests", || {
            for request in server.incoming_requests() {
                serve(request, &state, &front);
            }
        });
    })
}
