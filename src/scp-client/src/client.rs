//! Contains the implementation of ScpClient
//! # Examples
//! ```
//! use std::time::Duration;
//! let mut client = ScpClientBuilder::builder()
//! .audio_port(7001)
//! .port_scp(60102)
//! .build();
//! // got the address from mDNS browse
//! let addr = SocketAddr::new(IpAddr::from_str("192.168.8.106").unwrap(), 60102);
//! let config = client.request_chat(addr);
//! // use the config to listen to streams
//! std::thread::sleep(Duration::from_secs(1));
//! client.end_connection();
//!
//! if let Some(ip) = client.has_incoming_connections() {
//!    // either error or new SessionConfig
//!    let config = client.accept_incoming_connection().unwrap();
//!    // again, listen to the streams specified in config
//! }
//! ```
use std::fmt::Debug;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::misc;
use crate::scp::{ScpCommand, ScpMessage};

const TCP_TIMEOUT: Duration = Duration::from_secs(1);
/// Events used by the client to signify what happens inside the thread with the socket
#[derive(Debug)]
pub enum ConnectionEvent {
    /// Connection established. Sockets should be ready to receive data and transmit data
    ConnectionEstablished(SessionConfig),
    /// Connection failed - refused, busy or other
    ConnectionFailed(ScpConnectionError),
    /// Peer attempts to make a connection and waiting for confirmation
    ConnectionIncoming(IpAddr),
    /// Connection ended for whatever reason. Sockets should be cleaned up
    ConnectionEnd,
}
/// Events that can be emitted to the thread to make it take an action
#[derive(Debug)]
pub enum ConnectionAction {
    /// Attempt to make a connection with the provided settings
    AttemptConnection(ConnectionSetings),
    /// Refuse incoming connection, or do nothing if no incoming connections
    RefuseConnection,
    /// Accept incoming connection, or do nothing if no incoming connections
    AcceptConnection,
    /// Set password required for the connection, making an encryption key with it
    SetPassword(String),
    /// Remove the password for the socket connection, switching to automatic key generation
    UnsetPassword,
    EndConnection,
    Terminate,
}
/// Configuration for an established chat session
/// These are "suggestions" only and the responsibility to use all of them correctly
/// falls on the external implementation.
/// * `ip` - IpAddr of the connection
/// * `port_video` - UDP port to send video stream to
/// * `port_audio` - UDP port to send audio stream to
/// * `video_encoding` - !UNUSED! method of video encoding used
/// * `audio_encoding` - !UNUSED! method of audio encoding used
/// * `encryption_key` - encryption key used to encrypt all and any packets sent
/// * `encryption_method` - !UNUSED! - encryption method used
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub encryption_key: Option<String>,
    pub encrytpion_method: Option<bool>,
    pub ip: IpAddr,
    pub port_video: Option<u16>,
    pub port_audio: Option<u16>,
    pub video_encoding: VideoEncoding,
    pub audio_encoding: AudioEncoding,
}

/// Available video encoding formats
#[derive(Clone, Copy, Debug)]
pub enum VideoEncoding {
    H264,
}
/// Available audio encoding formats
#[derive(Clone, Copy, Debug)]
pub enum AudioEncoding {
    NoIdea,
}

use thiserror::Error;
#[derive(Clone, Copy, Debug, Error)]
/// Errors that may arise when establishing a session fails.
/// Some of the errors may require only small changes to provided config
pub enum ScpConnectionError {
    #[error("ScpClient not responding, either dead or unknown issue")]
    NotResponding,
    #[error("ScpClient not responded with busy. Try again later")]
    Busy,
    #[error("Peer refused the connection.")]
    Refused,
    #[error("Peer requires the connection to be initialized with a password")]
    PasswordRequired,
}
/// Preferences that ScpClient takes when etablishing a connection
#[derive(Clone, Copy, Debug)]
struct Preferences {
    video_encoding: VideoEncoding,
    audio_encoding: AudioEncoding,
    port_in_video: u16,
    port_in_audio: u16,
    port_scp: u16,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            video_encoding: VideoEncoding::H264,
            audio_encoding: AudioEncoding::NoIdea,
            port_in_audio: 7001,
            port_in_video: 7000,
            port_scp: 60201,
        }
    }
}

/// Settings used when attempting to make a connection to another ScpClient
#[derive(Debug)]
pub struct ConnectionSetings {
    pub destination: SocketAddr,
    pub password: Option<String>,
}

// What does the user want:
// 1. Try to connect with some settings
// 2. Wait patiently for some result (sync or async)

// 3. Get the SessionConfig or Error specyfing why the connection cannot be made
// Just that, all implementation is hidden otherwise

pub struct ScpClient {
    last_config: Option<SessionConfig>,
    thread: JoinHandle<()>,
    preferences: Preferences,
    tx: Arc<Mutex<Option<ConnectionAction>>>,
    rx: Arc<Mutex<Option<ConnectionEvent>>>,
}

impl ScpClient {
    /// # Panics
    /// Panics when a listener cannot be created on the given TCP port.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::with_preferences(Preferences::default())
    }
    /// # Panics
    /// Panics when a listener cannot be created on the given TCP port.
    fn with_preferences(preferences: Preferences) -> Self {
        let (join_handle, tx, rx) = Self::spawn_handler_thread(preferences);

        Self {
            last_config: None,
            thread: join_handle,
            preferences,
            tx,
            rx,
        }
    }
    /// Spawns the event loop with TCP socket, reading the messages and responding to external events.
    /// Model of communication:
    /// - Mutex for action that the thread with TcpListener must take
    /// - Mutex for event that the TcpListener thread has to tell.
    ///
    /// Messages might be missed, but the events and data flow is structured good enough to get some context,
    /// if any part keeps some state about what's happening.
    /// More importantly, it gives "async-ish" felling
    #[allow(clippy::type_complexity)]
    fn spawn_handler_thread(
        preferences: Preferences,
    ) -> (
        JoinHandle<()>,
        Arc<Mutex<Option<ConnectionAction>>>,
        Arc<Mutex<Option<ConnectionEvent>>>,
    ) {
        let action: Arc<Mutex<Option<ConnectionAction>>> = Arc::new(Mutex::new(None));
        let event: Arc<Mutex<Option<ConnectionEvent>>> = Arc::new(Mutex::new(None));

        // Get the address to bind the listener to
        let addr = misc::get_local_ip()
            .or_else(|| {
                log::warn!("No local address found for ScpClient. Using Loopback address.");
                Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
            })
            .unwrap();
        let sock_addr = SocketAddr::new(addr, preferences.port_scp);

        let listener = TcpListener::bind(sock_addr)
            .unwrap_or_else(|e| panic!("Cannot bind the listener to {sock_addr}.\n{e}"));

        let rx = Arc::clone(&action);
        let tx = Arc::clone(&event);
        listener.set_nonblocking(true).unwrap();

        let t = std::thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024);
            // Some internal state / context here
            // ScpListener struct or something to parse the messages
            loop {
                std::thread::sleep(Duration::from_millis(30));
                //check the actions if there are any
                let mut action = rx.lock().unwrap();
                if matches!(*action, Some(ConnectionAction::Terminate)) {
                    break;
                }
                if let Some(ConnectionAction::AttemptConnection(settings)) = &*action {
                    let mut stream =
                        TcpStream::connect_timeout(&settings.destination, TCP_TIMEOUT).unwrap();
                    stream
                        .write_all(&ScpMessage::new(ScpCommand::Start, b"L").as_bytes())
                        .unwrap();
                    *tx.lock().unwrap() =
                        Some(ConnectionEvent::ConnectionEstablished(SessionConfig {
                            encryption_key: None,
                            encrytpion_method: None,
                            ip: settings.destination.ip(),
                            port_video: Some(7000),
                            port_audio: None,
                            video_encoding: VideoEncoding::H264,
                            audio_encoding: AudioEncoding::NoIdea,
                        }));
                    stream.flush().unwrap();
                }
                *action = None; // take the action

                // Accept one connection.
                // The TCP stream is nonblocking, meaning it will return Err if no connections available
                // This way the loop continues
                let _ = listener.take_error();
                if let Ok((mut stream, addr_in)) = listener.accept() {
                    buf.resize(1024, 0);
                    if let Ok(size) = stream.read(&mut buf) {
                        if size == 0 {
                            continue;
                        }
                        let msg = ScpMessage::deserialize(&buf[..size]);
                        if msg.is_err() {
                            continue;
                        }
                        let msg = msg.unwrap();
                        if msg.command == ScpCommand::Start {
                            *tx.lock().unwrap() =
                                Some(ConnectionEvent::ConnectionEstablished(SessionConfig {
                                    encryption_key: None,
                                    encrytpion_method: None,
                                    ip: addr_in.ip(),
                                    port_video: Some(7000),
                                    port_audio: None,
                                    video_encoding: VideoEncoding::H264,
                                    audio_encoding: AudioEncoding::NoIdea,
                                }));
                        }
                    }
                }
            }
        });

        (t, action, event)
    }
    pub fn request_chat(
        &self,
        destination: SocketAddr,
    ) -> Result<SessionConfig, ScpConnectionError> {
        *self.tx.lock().unwrap() = Some(ConnectionAction::AttemptConnection(ConnectionSetings {
            destination,
            password: None,
        }));
        // check the connection with a timeout
        const TIMEOUT: Duration = Duration::from_secs(5);
        let start = std::time::Instant::now();
        while start + TIMEOUT > std::time::Instant::now() {
            let msg = self.rx.lock().unwrap();
            match &*msg {
                Some(ConnectionEvent::ConnectionEstablished(s)) => return Ok(s.clone()),
                Some(ConnectionEvent::ConnectionFailed(scp_connection_error)) => {
                    return Err(*scp_connection_error)
                }
                Some(ConnectionEvent::ConnectionEnd) => {
                    return Err(ScpConnectionError::NotResponding)
                }
                None => std::thread::sleep(Duration::from_millis(100)),
                _ => break,
            }
        }
        Err(ScpConnectionError::Refused)
    }

    pub fn has_incoming_connections(&mut self) -> Option<IpAddr> {
        if let Some(ConnectionEvent::ConnectionIncoming(addr)) = &*self.rx.lock().unwrap() {
            return Some(*addr);
        }

        None
    }
    pub fn accept_incoming_connection(&mut self) -> Result<SessionConfig, ScpConnectionError> {
        const TIMEOUT: Duration = std::time::Duration::from_secs(3);
        *self.tx.lock().unwrap() = Some(ConnectionAction::AcceptConnection);
        let start = std::time::Instant::now();
        while start + TIMEOUT > std::time::Instant::now() {
            match &*self.rx.lock().unwrap() {
                Some(ConnectionEvent::ConnectionEstablished(cfg)) => return Ok(cfg.clone()),
                Some(ConnectionEvent::ConnectionFailed(e)) => return Err(*e),
                _ => std::thread::sleep(Duration::from_millis(100)),
            }
        }

        Err(ScpConnectionError::NotResponding)
    }
    pub fn end_connection(&mut self) {
        *self.tx.lock().unwrap() = Some(ConnectionAction::EndConnection);
    }
}
impl Drop for ScpClient {
    fn drop(&mut self) {
        // if poisoned then thread already panicked and doesn't exist
        if !self.tx.is_poisoned() {
            *self.tx.lock().unwrap() = Some(ConnectionAction::Terminate);
        }
    }
}

/// Convinient builder for ScpClient with preferences
pub struct ScpClientBuilder {
    preferences: Preferences,
}

impl ScpClientBuilder {
    pub fn builder() -> Self {
        Self {
            preferences: Preferences::default(),
        }
    }

    /// # Panics
    /// May panic when the settings cannot be used: i.e. TCP port unavailable
    pub fn build(self) -> ScpClient {
        ScpClient::with_preferences(self.preferences)
    }
    pub fn video_port(self, port: u16) -> Self {
        Self {
            preferences: Preferences {
                port_in_video: port,
                ..self.preferences
            },
        }
    }
    pub fn audio_port(self, port: u16) -> Self {
        Self {
            preferences: Preferences {
                port_in_audio: port,
                ..self.preferences
            },
        }
    }
    pub fn video_encoding(self, encoding: VideoEncoding) -> Self {
        Self {
            preferences: Preferences {
                video_encoding: encoding,
                ..self.preferences
            },
        }
    }
    pub fn audio_encoding(self, encoding: AudioEncoding) -> Self {
        Self {
            preferences: Preferences {
                audio_encoding: encoding,
                ..self.preferences
            },
        }
    }
    pub fn port_scp(self, port: u16) -> Self {
        Self {
            preferences: Preferences {
                port_scp: port,
                ..self.preferences
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use crate::misc::get_local_ip;

    use super::{ScpClient, ScpClientBuilder};

    fn prepare_two_clients() -> (ScpClient, ScpClient) {
        let client = ScpClientBuilder::builder()
            .audio_port(7001)
            .port_scp(60102)
            .build();
        let client2 = ScpClientBuilder::builder()
            .audio_port(7001)
            .port_scp(60103)
            .build();
        (client, client2)
    }
    #[test]
    fn test_accept() {
        let (client1, mut client2) = prepare_two_clients();
        let ip = get_local_ip().unwrap();

        let addr = SocketAddr::new(ip, 60103);
        std::thread::sleep(Duration::from_millis(100));
        let config = client1.request_chat(addr);

        dbg!(&config);
        assert!(config.is_ok());
    }
}
