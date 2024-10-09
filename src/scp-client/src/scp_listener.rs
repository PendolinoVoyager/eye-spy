//! A wrapper around ScpListener inside a thread in ScpClient.
//! It manages internal state, listens to ConnectionAction events it has to respond to
//! and emits ConnectionEvent when something happens.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::Deserializer;

use crate::client::{
    ActionConnector, ConnectionAction, ConnectionEvent, ConnectionSetings, EventConnector,
    Preferences, SessionConfig,
};
use crate::misc::{self};
use crate::scp::{ScpCommand, ScpMessage};
const TCP_TIMEOUT: Duration = Duration::from_secs(1);
const EVENT_LOOP_MIN_TIME: Duration = Duration::from_millis(30);
/// The current state of the connection.
/// In an ideal world, it should go from top to bottom
#[derive(PartialEq, Debug, Clone, Copy)]
enum ConnectionState {
    /// Not connecting anywhere
    Free,
    /// Performing a handshake - initialized from either side
    Handshake,
    /// Awaiting the confirmation - either waiting for user to accept or peer to do so
    ConfigShared,
    Awaiting,
    /// Connection fully established
    Connected,
}
/// An overly complex state machine that manages the connection with Scp protocol
#[derive(Debug)]
pub struct ScpListener {
    action: ActionConnector,
    event: EventConnector,
    communicating_with: Option<SocketAddr>,
    got_preferences: Option<Preferences>,
    state: ConnectionState,
    preferences: Preferences,
    pub tcp_listener: TcpListener,
    buf: Vec<u8>,
}
impl ScpListener {
    pub fn new(
        action: ActionConnector,
        event: EventConnector,
        mut preferences: Preferences,
    ) -> Self {
        let addr = misc::get_local_ip()
            .or_else(|| {
                log::warn!("No local address found for ScpClient. Using Loopback address.");
                Some(IpAddr::V4(Ipv4Addr::LOCALHOST))
            })
            .unwrap();
        let sock_addr = SocketAddr::new(addr, preferences.port_scp);
        let listener = TcpListener::bind(sock_addr)
            .unwrap_or_else(|e| panic!("Cannot bind the listener to {sock_addr}.\n{e}"));

        // The OS might have given us a different port when the preferences are set to 0
        preferences.port_scp = listener.local_addr().unwrap().port();

        listener.set_nonblocking(true).unwrap();
        Self {
            action,
            event,
            preferences,
            communicating_with: None,
            got_preferences: None,
            state: ConnectionState::Free,
            tcp_listener: listener,
            buf: Vec::with_capacity(1024),
        }
    }
    pub fn handle_event_loop(&mut self) -> anyhow::Result<()> {
        // Check the action that need to be taken first
        // The function call shouldn't be expensive as there's a potential connection waiting next
        let start = Instant::now();
        self.handle_action()?;

        // Handle any incoming connection
        self.handle_connection()?;
        self.buf.clear();
        let diff = Instant::now().duration_since(start);
        if diff < EVENT_LOOP_MIN_TIME {
            std::thread::sleep(EVENT_LOOP_MIN_TIME - diff);
        }
        Ok(())
    }
    /// Handle the action from ConnectionAction.
    /// If the action
    fn handle_action(&mut self) -> anyhow::Result<()> {
        let mut action = self.action.0.lock().unwrap();
        if action.is_none() {
            return Ok(());
        }
        // unfortunate clone thanks to the borrow checker
        let cloned = action.as_ref().unwrap().clone();
        action.take();
        drop(action);

        match cloned {
            ConnectionAction::AttemptConnection(settings) => {
                self.on_attempt_connection_action(&settings)
            }
            ConnectionAction::RefuseConnection => self.end_connection(),
            ConnectionAction::AcceptConnection => {
                if self.communicating_with.is_some() {
                    self.share_config();
                    self.finalize_connection();
                }
            }
            ConnectionAction::SetPassword(_) => todo!(),
            ConnectionAction::UnsetPassword => todo!(),
            ConnectionAction::EndConnection => self.end_connection(),
            ConnectionAction::Terminate => {
                self.end_connection();
                *self.event.0.lock().unwrap() = None;

                self.event.1.notify_one();
                return Err(anyhow::Error::msg("ScpListener terminated properly."));
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
            if let Ok(size) = stream.read_to_end(&mut self.buf) {
                if size == 0 {
                    return Ok(());
                }

                let msg = ScpMessage::deserialize(&self.buf[..size]).unwrap();
                let _ = stream.flush();
                self.handle_scp_message(msg, addr_in);
            }
        }
        Ok(())
    }

    // Following parts are just handlers for each action, should be inlined really
    fn on_attempt_connection_action(&mut self, settings: &ConnectionSetings) {
        if self.state == ConnectionState::Connected {
            *self.event.0.lock().unwrap() = Some(ConnectionEvent::ConnectionFailed(
                crate::client::ScpConnectionError::AlreadyConnected,
            ));
            self.event.1.notify_one();
            return;
        }
        let mut stream = TcpStream::connect_timeout(&settings.destination, TCP_TIMEOUT).unwrap();
        stream
            .write_all(
                &ScpMessage::new(ScpCommand::Start, &self.preferences.port_scp.to_le_bytes())
                    .as_bytes(),
            )
            .unwrap();
        self.communicating_with = Some(settings.destination);
        self.state = ConnectionState::Handshake;
    }
    fn handle_scp_message(&mut self, msg: ScpMessage, addr_in: SocketAddr) {
        match msg.command {
            ScpCommand::Start => self.init_connection(msg, addr_in),
            ScpCommand::OwnKeyRequired => todo!(),
            ScpCommand::ReqGenerateKey => todo!(),
            ScpCommand::AckGenerateKey => todo!(),
            ScpCommand::KeyShare => todo!(),
            ScpCommand::PreferencesShare => self.on_preferences_share(msg),
            ScpCommand::Ready => self.finalize_connection(),
            ScpCommand::SimpleMessage => todo!(),
            ScpCommand::End => {
                self.notify_end_connection();
            }
        }
    }
    fn end_connection(&mut self) {
        if let Some(sock_addr) = self.communicating_with {
            if let Ok(mut stream) = TcpStream::connect(sock_addr) {
                let _ = stream.set_nonblocking(true);
                let _ = stream.write(&ScpMessage::new(ScpCommand::End, b"").as_bytes());
            }
        }
    }
    fn notify_end_connection(&mut self) {
        *self.event.0.lock().unwrap() = Some(ConnectionEvent::ConnectionEnd);
        self.event.1.notify_one();
        self.communicating_with = None;
        self.got_preferences = None;
    }
    /// Called when a connection comes from the peer first
    fn init_connection(&mut self, msg: ScpMessage, addr_in: SocketAddr) {
        if self.state != ConnectionState::Free {
            self.end_connection();
        }
        if msg.body.len() >= 2 {
            let slice = &msg.body[0..2];
            if let Ok(port) = slice.try_into().map(u16::from_le_bytes) {
                self.communicating_with = Some(SocketAddr::new(addr_in.ip(), port));
                self.share_config();
                self.state = ConnectionState::ConfigShared;
            }
        }
    }

    fn on_preferences_share(&mut self, msg: ScpMessage) {
        // Get the shared preferences
        // If we have shared, we can see ourselves as connected
        // Why serde and json right now from all places? I was lazy
        let mut deser = Deserializer::from_slice(&msg.body);
        let preferences = Preferences::deserialize(&mut deser);
        if let Ok(p) = preferences {
            self.got_preferences = Some(p);
            match self.state {
                ConnectionState::Handshake => self.share_config(),
                ConnectionState::ConfigShared => {
                    let _ = TcpStream::connect(self.communicating_with.unwrap())
                        .unwrap()
                        .write(&ScpMessage::new(ScpCommand::Ready, b"").as_bytes());
                    self.state = ConnectionState::Awaiting;
                }
                ConnectionState::Awaiting => self.finalize_connection(),
                _ => (),
            }
        } else {
            self.end_connection();
        }
    }

    /// Share the if addr_in is present
    /// Change the state to ConfigShared
    fn share_config(&mut self) {
        // share your config
        if let Some(addr_in) = self.communicating_with {
            let t = serde_json::to_vec(&self.preferences);
            if t.is_err() {
                self.end_connection();
            }
            let _ = TcpStream::connect(addr_in)
                .unwrap()
                .write(&ScpMessage::new(ScpCommand::PreferencesShare, &t.unwrap()).as_bytes());
            self.state = ConnectionState::ConfigShared;
        }
    }
    /// Function to call when we're ready to receive data from a peer
    fn finalize_connection(&mut self) {
        *self.event.0.lock().unwrap() =
        Some(ConnectionEvent::ConnectionEstablished(SessionConfig {
            encryption_key: None,
            encrytpion_method: None,
            ip: self.communicating_with.expect("Invalid finalize connection call. Expected to have a peer communicating with, got None.").ip(),
            stream_config: self.got_preferences.expect("Cannot finalize connection with no preferences"),
        }));
        self.event.1.notify_one();
        self.state = ConnectionState::Connected;
    }
}
