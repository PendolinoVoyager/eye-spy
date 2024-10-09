//! This module controls how the connection is established, controlled and switches state
//! from the level of Bevy. The elements are in place, but need to be wrapped in bevy ECS to work with UI.
use std::net::IpAddr;

use bevy::prelude::*;
use scp_client::client::SessionConfig;

use crate::h264_stream::incoming::{H264IncomingStreamControls, IncomingStreamControls};
use crate::h264_stream::outgoing::{H264StreamControls, StreamControls};
use crate::{IncomingVideoStreamControls, OutgoingVideoStreamControls, STREAM_IMAGE_HANDLE};

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutgoingVideoStreamState {
    On,
    #[default]
    Off,
}
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum IncomingVideoStreamState {
    On,
    #[default]
    Off,
}
/// The connection state of ScpClient
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ScpConnectionState {
    #[default]
    Off,
    Connecting,
    Connected,
}

#[derive(Event)]
pub struct ConnectionEvent(SessionConfig);
#[derive(Event)]
pub struct IncomingConnectionEvent(IpAddr);

pub struct ConnectionStatePlugin;

impl Plugin for ConnectionStatePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<OutgoingVideoStreamState>();
        app.init_state::<IncomingVideoStreamState>();
        app.init_state::<ScpConnectionState>();
        app.add_event::<ConnectionEvent>();
        app.add_event::<IncomingConnectionEvent>();

        app.add_systems(
            OnEnter(OutgoingVideoStreamState::Off),
            on_disconnect_out_stream,
        );
        app.add_systems(
            OnEnter(IncomingVideoStreamState::Off),
            on_disconnect_in_stream,
        );

        app.add_systems(
            OnTransition {
                exited: ScpConnectionState::Connecting,
                entered: ScpConnectionState::Off,
            },
            on_fail_connection,
        );
    }
}

// CHANGING STATE SYSTEMS, TODO

fn on_disconnect_out_stream(mut os: ResMut<OutgoingVideoStreamControls<H264StreamControls>>) {
    os.0.disconnect();
}
fn on_disconnect_in_stream(
    mut is: ResMut<IncomingVideoStreamControls<H264IncomingStreamControls>>,
    mut images: ResMut<Assets<Image>>,
) {
    is.0.refuse();
    if let Some(image) = images.get_mut(&STREAM_IMAGE_HANDLE) {
        image.data.iter_mut().for_each(|e| *e = 0u8);
    }
}

fn on_fail_connection() {
    warn!("Failed a connection.");
}
fn on_connection_event() {
    // init the streams
    // change state to connected
}

fn on_disconnect_event() {
    // disconnect the streams
    // change the state to disconnected
}
fn on_incoming_connection_event() {
    // Show the buttons at the top
    // When buttons are clicked they disconnect current session and accept connection in a task
    // When the task completes it emits a ConnectionEvent
}

// fn on_connection_event(
//     query: Query<(&Interaction, &HostButton), Changed<Interaction>>,
//     mut v_stream: ResMut<OutgoingVideoStreamControls<H264StreamControls>>,
//     mut stream_in_state: ResMut<NextState<IncomingVideoStreamState>>,
//     mut stream_out_state: ResMut<NextState<OutgoingVideoStreamState>>,
//     mut incoming: ResMut<IncomingVideoStreamControls<H264IncomingStreamControls>>,
// ) {
//     v_stream.0.connect(sock_addr); // Outgoing - sending to 7000
//     let mut in_addr = sock_addr;
//     in_addr.set_port(6969);
//     incoming.0.accept(in_addr).unwrap(); // incoming - connecting to 6969
// }
