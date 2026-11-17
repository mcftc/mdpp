//! Single instance (PLAN §4.1): a second `mdpp` hands its files to the running one and exits.
//!
//! The running instance listens on a per-user local socket (Linux abstract namespace, Windows
//! named pipe — neither leaves a stale file behind after a crash). A message is the requests as
//! TOML with a 4-byte length prefix; the reply is one byte, sent once the requests were handed on.

use std::io::{self, Read, Write};
use std::thread;

use interprocess::local_socket::prelude::*;
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Name};
use serde::{Deserialize, Serialize};

use crate::cli::OpenRequest;

/// Messages larger than this are refused; a list of paths is far smaller.
const MAX_MESSAGE_BYTES: u32 = 1 << 20;
const ACK: u8 = 1;

/// Outcome of trying to become the one running instance.
#[derive(Debug)]
pub enum Claim {
    /// Another instance took the files; this process should exit.
    Forwarded,
    /// This process is the instance; serve forwarded requests.
    Primary(Server),
    /// Single instance is not available here; run standalone.
    Unavailable(io::Error),
}

#[derive(Debug)]
pub struct Server {
    listener: LocalSocketListener,
}

#[derive(Serialize, Deserialize)]
struct Message {
    #[serde(default)]
    request: Vec<OpenRequest>,
}

/// The socket name for the current user.
pub fn default_name() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let user: String = user.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("mdpp-{user}.sock")
}

/// Hand `requests` to a running instance, or become the running instance.
pub fn claim(name: &str, requests: &[OpenRequest]) -> Claim {
    let socket = match socket_name(name) {
        Ok(socket) => socket,
        Err(error) => return Claim::Unavailable(error),
    };
    if forward(&socket, requests).is_ok() {
        return Claim::Forwarded;
    }
    match ListenerOptions::new().name(socket.clone()).create_sync() {
        Ok(listener) => Claim::Primary(Server { listener }),
        // Lost a start-up race: the other process just became the instance.
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            match forward(&socket, requests) {
                Ok(()) => Claim::Forwarded,
                Err(error) => Claim::Unavailable(error),
            }
        }
        Err(error) => Claim::Unavailable(error),
    }
}

fn socket_name(name: &str) -> io::Result<Name<'static>> {
    name.to_string().to_ns_name::<GenericNamespaced>()
}

fn forward(socket: &Name<'_>, requests: &[OpenRequest]) -> io::Result<()> {
    let mut stream = LocalSocketStream::connect(socket.borrow())?;
    let body = toml::to_string(&Message {
        request: requests.to_vec(),
    })
    .map_err(io::Error::other)?;
    let length = u32::try_from(body.len()).map_err(io::Error::other)?;
    stream.write_all(&length.to_le_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()?;
    let mut ack = [0u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] == ACK {
        Ok(())
    } else {
        Err(io::Error::other("unexpected reply"))
    }
}

impl Server {
    /// Accept forwarded requests on a background thread; `on_open` runs on that thread.
    pub fn serve(self, on_open: impl Fn(Vec<OpenRequest>) + Send + 'static) {
        thread::Builder::new()
            .name("mdpp-instance".into())
            .spawn(move || {
                for connection in self.listener.incoming() {
                    let Ok(mut stream) = connection else {
                        continue;
                    };
                    if let Ok(requests) = receive(&mut stream) {
                        on_open(requests);
                        let _ = stream.write_all(&[ACK]);
                    }
                }
            })
            .expect("spawn the single-instance thread");
    }
}

fn receive(stream: &mut LocalSocketStream) -> io::Result<Vec<OpenRequest>> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length)?;
    let length = u32::from_le_bytes(length);
    if length > MAX_MESSAGE_BYTES {
        return Err(io::Error::other("message too large"));
    }
    let mut body = vec![0u8; length as usize];
    stream.read_exact(&mut body)?;
    let body = String::from_utf8(body).map_err(io::Error::other)?;
    let message: Message = toml::from_str(&body).map_err(io::Error::other)?;
    Ok(message.request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn second_instance_forwards_to_the_first() {
        let name = format!("mdpp-test-{}.sock", std::process::id());
        let Claim::Primary(server) = claim(&name, &[]) else {
            panic!("the first claim must become the instance");
        };
        let (sent, received) = mpsc::channel();
        server.serve(move |requests| sent.send(requests).unwrap());

        let request = OpenRequest {
            path: "/tmp/ğüş notes.md".into(),
            line: Some(3),
            column: None,
        };
        assert!(matches!(
            claim(&name, std::slice::from_ref(&request)),
            Claim::Forwarded
        ));
        let got = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(got, vec![request]);
    }
}
