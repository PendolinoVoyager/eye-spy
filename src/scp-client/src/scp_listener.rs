//! A wrapper around ScpListener inside a thread in ScpClient.
//! It manages internal state, listens to ConnectionAction events it has to respond to
//! and emits ConnectionEvent when something happens.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

use crate::client::{
    ActionConnector, AudioEncoding, ConnectionAction, ConnectionEvent, ConnectionSetings,
    EventConnector, SessionConfig, VideoEncoding,
};
use crate::misc;
use crate::scp::{ScpCommand, ScpMessage};
const TCP_TIMEOUT: Duration = Duration::from_secs(1);
/// The current state of the connection.
/// In an ideal world, it should go from top to bottom
#[derive(PartialEq)]
enum ConnectionState {
    /// Not connecting anywhere
    Free,
    /// Performing a handshake - initialized from either side
    Handshake,
    /// Awaiting the confirmation - either waiting for user to accept or peer to do so
    Awaiting,
    /// Connection fully established
    Connected,
}
/// An overly complex state machine that manages the connection with Scp protocol
pub struct ScpListener {
    action: ActionConnector,
    event: EventConnector,
    communicating_with: Option<SocketAddr>,
    state: ConnectionState,
    tcp_listener: TcpListener,
    buf: Vec<u8>,
}
impl ScpListener {
    pub fn new(action: ActionConnector, event: EventConnector, port: u16) -> Self {
        let addr = misc::get_local_ip()
            .or_else(|| {
                log::warn!("No local address found for ScpClient. Using Loopback address.");
                Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
            })
            .unwrap();
        let sock_addr = SocketAddr::new(addr, port);
        let listener = TcpListener::bind(sock_addr)
            .unwrap_or_else(|e| panic!("Cannot bind the listener to {sock_addr}.\n{e}"));

        listener.set_nonblocking(true).unwrap();
        Self {
            action,
            event,
            communicating_with: None,
            state: ConnectionState::Free,
            tcp_listener: listener,
            buf: Vec::with_capacity(1024),
        }
    }
    pub fn handle_event_loop(&mut self) -> anyhow::Result<()> {
        // Check the action that need to be taken first
        // The function call shouldn't be expensive as there's a potential connection waiting next

        self.handle_action()?;

        // Handle any incoming connection
        self.handle_connection()?;

        Ok(())
    }
    /// Handle the action from ConnectionAction.
    /// If the action
    fn handle_action(&mut self) -> anyhow::Result<()> {
        let action = self.action.0.lock().unwrap();
        if action.is_none() {
            return Ok(());
        }

        let cloned = action.as_ref().unwrap().clone();
        drop(action);

        match cloned {
            ConnectionAction::AttemptConnection(settings) => {
                self.on_attempt_connection_action(&settings)
            }
            ConnectionAction::RefuseConnection => self.end_connection(),
            ConnectionAction::AcceptConnection => todo!(),
            ConnectionAction::SetPassword(_) => todo!(),
            ConnectionAction::UnsetPassword => todo!(),
            ConnectionAction::EndConnection => todo!(),
            ConnectionAction::Terminate => {
                return Err(anyhow::Error::msg("ScpListener terminated properly."))
            }
        };
        Ok(())
    }
    /// Handle the incoming connection. If none are present, skip
    /// If returns error, pass it down to the event loop handler
    fn handle_connection(&mut self) -> anyhow::Result<()> {
        if let Ok((mut stream, addr_in)) = self.tcp_listener.accept() {
            // If we are in the middle of smthng and the connection comes from somewhere else:
            if self.state != ConnectionState::Free
                && self
                    .communicating_with
                    .is_some_and(|sa| sa.ip() != addr_in.ip())
            {
                let _ =
                    stream.write(&ScpMessage::new(ScpCommand::End, b"I'my busy my man").as_bytes());

                return Ok(());
            }

            if let Ok(size) = stream.read(&mut self.buf) {
                if size == 0 {
                    return Ok(());
                }
                let msg = ScpMessage::deserialize(&self.buf[..size]).unwrap();
                self.handle_scp_message(stream, addr_in, msg);
            }
        }
        Ok(())
    }

    // Following parts are just handlers for each action
    fn on_attempt_connection_action(&self, settings: &ConnectionSetings) {
        if self.state == ConnectionState::Connected {
            *self.event.0.lock().unwrap() = Some(ConnectionEvent::ConnectionFailed(
                crate::client::ScpConnectionError::AlreadyConnected,
            ));
            self.event.1.notify_one();
            return;
        }
        let mut stream = TcpStream::connect_timeout(&settings.destination, TCP_TIMEOUT).unwrap();
        stream
            .write_all(&ScpMessage::new(ScpCommand::Start, b"L").as_bytes())
            .unwrap();

        let mut event = self.event.0.lock().unwrap();
        *event = Some(ConnectionEvent::ConnectionEstablished(SessionConfig {
            encryption_key: None,
            encrytpion_method: None,
            ip: settings.destination.ip(),
            port_video: Some(7000),
            port_audio: None,
            video_encoding: VideoEncoding::H264,
            audio_encoding: AudioEncoding::NoIdea,
        }));

        self.event.1.notify_one();
    }
    fn handle_scp_message(&mut self, stream: TcpStream, addr_in: SocketAddr, msg: ScpMessage) {
        if msg.command == ScpCommand::Start {
            let mut event = self.event.0.lock().unwrap();
            *event = Some(ConnectionEvent::ConnectionEstablished(SessionConfig {
                encryption_key: None,
                encrytpion_method: None,
                ip: addr_in.ip(),
                port_video: Some(7000),
                port_audio: None,
                video_encoding: VideoEncoding::H264,
                audio_encoding: AudioEncoding::NoIdea,
            }));
            self.event.1.notify_one();
        }
    }
    fn end_connection(&mut self) {
        if let Some(sock_addr) = self.communicating_with {
            if let Ok(mut stream) = TcpStream::connect(sock_addr) {
                let _ = stream.write(&ScpMessage::new(ScpCommand::End, b"").as_bytes());
                let _ = stream.flush();
            }
            *self.event.0.lock().unwrap() = Some(ConnectionEvent::ConnectionEnd);
            self.event.1.notify_one();
            self.communicating_with = None;
        }
    }
}
