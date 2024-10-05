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
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
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
    tx: Sender<ConnectionAction>,
    rx: Receiver<ConnectionEvent>,
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
    /// - MPSC channel to send new Events (connect, refuse, set password, etc)
    /// - Other MPSC channel to receive what the thread has to say (established connection, errored, or got request)
    fn spawn_handler_thread(
        preferences: Preferences,
    ) -> (
        JoinHandle<()>,
        Sender<ConnectionAction>,
        Receiver<ConnectionEvent>,
    ) {
        let (tx_a, rx_a) = mpsc::channel::<ConnectionAction>();
        let (tx_e, rx_e) = mpsc::channel::<ConnectionEvent>();

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

        listener.set_nonblocking(true).unwrap();
        let t = std::thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::with_capacity(1024);
            // Some internal state / context here
            // ScpListener struct or something to parse the messages
            loop {
                //check the actions if there are any
                for action in rx_a.iter() {
                    if let ConnectionAction::AttemptConnection(settings) = action {
                        let size = TcpStream::connect_timeout(&settings.destination, TCP_TIMEOUT)
                            .unwrap()
                            .read_to_end(&mut buf)
                            .unwrap();
                        let msg = ScpMessage::deserialize(&buf[..size]).unwrap();
                        dbg!(msg);
                        tx_e.send(ConnectionEvent::ConnectionEstablished(SessionConfig {
                            encryption_key: None,
                            encrytpion_method: None,
                            ip: settings.destination.ip(),
                            port_video: Some(7000),
                            port_audio: None,
                            video_encoding: VideoEncoding::H264,
                            audio_encoding: AudioEncoding::NoIdea,
                        }))
                        .unwrap();
                    }
                }
                // Accept one connection.
                // The TCP stream is nonblocking, meaning it will return Err if no connections available
                // This way the loop continues
                let _ = listener.take_error();
                if let Ok((mut stream, addr_in)) = listener.accept() {
                    dbg!("Accepted");

                    if let Ok(size) = stream.read_to_end(&mut buf) {
                        let msg = ScpMessage::deserialize(&buf[..size]);
                        if msg.is_err() {
                            continue;
                        }
                        let msg = msg.unwrap();
                        if msg.command == ScpCommand::Start {
                            tx_e.send(ConnectionEvent::ConnectionEstablished(SessionConfig {
                                encryption_key: None,
                                encrytpion_method: None,
                                ip: addr_in.ip(),
                                port_video: Some(7000),
                                port_audio: None,
                                video_encoding: VideoEncoding::H264,
                                audio_encoding: AudioEncoding::NoIdea,
                            }))
                            .unwrap();
                        }
                    }
                }
            }
        });

        (t, tx_a, rx_e)
    }
    pub fn request_chat(
        &self,
        destination: SocketAddr,
    ) -> Result<SessionConfig, ScpConnectionError> {
        self.tx
            .send(ConnectionAction::AttemptConnection(ConnectionSetings {
                destination,
                password: None,
            }))
            .unwrap();
        while let Ok(msg) = self.rx.recv_timeout(Duration::from_secs(1)) {
            match msg {
                ConnectionEvent::ConnectionEstablished(s) => return Ok(s),
                ConnectionEvent::ConnectionFailed(scp_connection_error) => {
                    return Err(scp_connection_error)
                }
                ConnectionEvent::ConnectionEnd => return Err(ScpConnectionError::NotResponding),
                _ => (),
            }
        }
        Err(ScpConnectionError::Refused)
    }
    pub fn events(&mut self) {
        for event in self.rx.iter() {}
    }
    pub fn has_incoming_connections(&mut self) -> Option<IpAddr> {
        for event in self.rx.iter() {
            if let ConnectionEvent::ConnectionIncoming(addr) = event {
                return Some(addr);
            }
        }
        None
    }
    pub fn accept_incoming_connection(&mut self) -> Result<SessionConfig, ScpConnectionError> {
        let _ = self.tx.send(ConnectionAction::AcceptConnection);
        while let Ok(event) = self.rx.recv_timeout(Duration::from_secs(3)) {
            match event {
                ConnectionEvent::ConnectionEstablished(session_config) => {
                    return Ok(session_config)
                }
                ConnectionEvent::ConnectionFailed(scp_connection_error) => {
                    return Err(scp_connection_error)
                }
                _ => (),
            }
        }
        Err(ScpConnectionError::NotResponding)
    }
    pub fn end_connection(&mut self) {
        let _ = self.tx.send(ConnectionAction::EndConnection);
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
    use std::net::{IpAddr, SocketAddr};
    use std::str::FromStr;
    use std::time::Duration;

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

        let addr = SocketAddr::new(IpAddr::from_str("127.0.0.1").unwrap(), 60103);
        std::thread::sleep(Duration::from_millis(100));
        let config = client1.request_chat(addr);
        std::thread::sleep(Duration::from_millis(100));

        assert!(client2.has_incoming_connections().is_some());
        dbg!(&config);
        assert!(config.is_ok());
        // use the config to listen to streams
        std::thread::sleep(std::time::Duration::from_secs(1));
        // client.end_connection();
        if let Some(ip) = client2.has_incoming_connections() {
            // either error or new SessionConfig
            let config = client2.accept_incoming_connection().unwrap();
            // again, listen to the streams specified in config
        }
    }
}
