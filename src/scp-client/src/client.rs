//! Contains the implementation of ScpClient
//! # Examples
//! ```
//! use std::time::Duration;
//! use scp_client::client::ScpClientBuilder;
//! use std::net::IpAddr;
//! use std::net::SocketAddr;
//! use std::str::FromStr;
//!
//! let mut client = ScpClientBuilder::builder()
//! .audio_port(7001)
//! .port_scp(60102)
//! .build();
//! let  _client2 = ScpClientBuilder::builder()
//! .audio_port(7001)
//! .port_scp(60103)
//! .build();
//! // got the address from mDNS browse
//! let addr = SocketAddr::new(IpAddr::from_str("192.168.8.106").unwrap(), 60103);
//! let config = client.request_chat(addr);
//! // use the config to listen to streams
//!
//! std::thread::sleep(Duration::from_millis(100));
//! client.end_connection();
//!
//! ```
use std::fmt::Debug;
use std::net::{IpAddr, SocketAddr};
use std::sync::Weak;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::scp_listener::ScpListener;

/// Events used by the client to signify what happens inside the thread with the socket
#[derive(Debug, Clone)]
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
#[derive(Debug, Clone)]
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
    pub(crate) stream_config: Preferences,
}

/// Available video encoding formats
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum VideoEncoding {
    H264,
}
/// Available audio encoding formats
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AudioEncoding {
    NoIdea,
}

use serde::{Deserialize, Serialize};
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
    #[error("ScpClient is already connected somewhere")]
    AlreadyConnected,
}

/// Preferences that ScpClient takes when etablishing a connection

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Preferences {
    pub video_encoding: VideoEncoding,
    pub audio_encoding: AudioEncoding,
    pub port_in_video: u16,
    pub port_in_audio: u16,
    pub port_scp: u16,
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
#[derive(Debug, Clone)]
pub struct ConnectionSetings {
    pub destination: SocketAddr,
    pub password: Option<String>,
}

pub type ActionConnector = Arc<(Mutex<Option<ConnectionAction>>, Condvar)>;
pub type EventConnector = Arc<(Mutex<Option<ConnectionEvent>>, Condvar)>;
// What does the user want:
// 1. Try to connect with some settings
// 2. Wait patiently for some result (sync or async)

// 3. Get the SessionConfig or Error specyfing why the connection cannot be made
// Just that, all implementation is hidden otherwise

pub struct ScpClient {
    preferences: Preferences,
    tx: ActionConnector,
    rx: EventConnector,
    sock_addr: SocketAddr,
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
        let (tx, rx, sock_addr) = Self::spawn_handler_thread(preferences);

        Self {
            preferences,
            tx,
            rx,
            sock_addr,
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
    fn spawn_handler_thread(
        preferences: Preferences,
    ) -> (ActionConnector, EventConnector, SocketAddr) {
        let action: ActionConnector = Arc::new((Mutex::new(None), Condvar::new()));
        let event: EventConnector = Arc::new((Mutex::new(None), Condvar::new()));

        let rx = Arc::clone(&action);
        let tx = Arc::clone(&event);

        let mut listener = ScpListener::new(rx, tx, preferences);
        let sock_addr = listener.tcp_listener.local_addr().unwrap();
        std::thread::spawn(move || 'outer: loop {
            match listener.handle_event_loop() {
                Ok(()) => continue,
                Err(e) => {
                    println!("{e}");

                    break 'outer;
                }
            }
        });

        (action, event, sock_addr)
    }

    pub fn request_chat(
        &self,
        destination: SocketAddr,
    ) -> Result<SessionConfig, ScpConnectionError> {
        *self.tx.0.lock().unwrap() = Some(ConnectionAction::AttemptConnection(ConnectionSetings {
            destination,
            password: None,
        }));
        self.tx.1.notify_all();

        let (lock, cvar) = &*self.rx;
        let msg = cvar
            .wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(5), |msg| {
                msg.is_none()
            })
            .unwrap()
            .0;

        match &*msg {
            Some(ConnectionEvent::ConnectionEstablished(s)) => Ok(s.clone()),
            Some(ConnectionEvent::ConnectionFailed(err)) => Err(*err),
            _ => Err(ScpConnectionError::NotResponding),
        }
    }
    /// Read the events in a blocking way
    pub fn events(&self) -> impl Iterator<Item = ConnectionEvent> {
        EventIterator {
            rx: Arc::downgrade(&self.rx),
        }
    }
    /// Blocking function. If no incoming connections, it will wait until there's one available.
    pub fn accept_incoming_connection(&mut self) -> Result<SessionConfig, ScpConnectionError> {
        const TIMEOUT: Duration = std::time::Duration::from_secs(3);
        *self.tx.0.lock().unwrap() = Some(ConnectionAction::AcceptConnection);
        self.tx.1.notify_all();
        let (lock, cvar) = &*self.rx;
        let _ = cvar.wait_timeout_while(lock.lock().unwrap(), TIMEOUT, |event| event.is_none());
        let val = &*lock.lock().unwrap();

        match val {
            Some(ConnectionEvent::ConnectionEstablished(s)) => Ok(s.clone()),
            Some(ConnectionEvent::ConnectionFailed(e)) => Err(*e),
            _ => Err(ScpConnectionError::NotResponding),
        }
    }
    pub fn end_connection(&mut self) {
        *self.tx.0.lock().unwrap() = Some(ConnectionAction::EndConnection);
    }
}
impl Drop for ScpClient {
    fn drop(&mut self) {
        // if poisoned then thread already panicked and doesn't exist
        if !self.tx.0.is_poisoned() {
            *self.tx.0.lock().unwrap() = Some(ConnectionAction::Terminate);
        }
        let _ = self;
    }
}
struct EventIterator {
    rx: Weak<(Mutex<Option<ConnectionEvent>>, Condvar)>,
}

impl Iterator for EventIterator {
    type Item = ConnectionEvent;

    fn next(&mut self) -> Option<Self::Item> {
        // Try to upgrade the Weak reference to Arc.

        // Lock and wait for a new event, as long as the message is None.
        let msg: Option<ConnectionEvent> = loop {
            let rx = self.rx.upgrade()?;
            let (lock, cvar) = &*rx;
            let msg =
                cvar.wait_timeout_while(lock.lock().unwrap(), Duration::from_secs(1), |msg| {
                    msg.is_none()
                });
            let res = match msg {
                Ok(v) if v.1.timed_out() => continue,
                Ok(v) => v.0.clone(),

                Err(_) => None,
            };
            break res;
        };

        // Clone the message and take it
        msg
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
    use std::time::Duration;

    use super::{ConnectionEvent, ScpClient, ScpClientBuilder};
    fn prepare_two_clients() -> (ScpClient, ScpClient) {
        let client = ScpClientBuilder::builder()
            .audio_port(7001)
            .port_scp(0)
            .build();
        let client2 = ScpClientBuilder::builder()
            .audio_port(7001)
            .port_scp(0)
            .build();
        (client, client2)
    }
    #[test]
    fn test_accept() {
        let (client1, mut client2) = prepare_two_clients();

        let addr = client2.sock_addr;
        std::thread::sleep(Duration::from_millis(100));
        let config = client1.request_chat(addr);
        std::thread::sleep(Duration::from_millis(300));
        let config2 = client2.accept_incoming_connection();
        assert!(config.is_ok());
        assert!(config2.is_ok());
    }
    #[test]
    fn test_event_iterator() {
        let (client1, mut client2) = prepare_two_clients();
        let iterator = client1.events();

        let handle = std::thread::spawn(move || {
            for event in iterator {
                if !matches!(event, ConnectionEvent::ConnectionEstablished(_)) {
                    return;
                }
            }
        });
        client1.request_chat(client2.sock_addr).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        client2.accept_incoming_connection().unwrap();
        std::thread::sleep(Duration::from_millis(1000));

        client2.end_connection();
        drop(client1);
        let _ = handle.join();
    }
    #[test]
    fn test_iterator_timeout() {
        let (client1, mut client2) = prepare_two_clients();
        let iterator = client1.events();

        let handle = std::thread::spawn(move || {
            for event in iterator {
                if !matches!(event, ConnectionEvent::ConnectionEstablished(_)) {
                    return;
                }
            }
        });
        client1.request_chat(client2.sock_addr).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        client2.accept_incoming_connection().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        client2.end_connection();
        // No events for 10 secs
        std::thread::sleep(Duration::from_millis(10000));
        drop(client1);
        let _ = handle.join();
    }
}
