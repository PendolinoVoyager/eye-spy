//! Module for UI states and logic.

use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};

use bevy::prelude::*;

use crate::h264_stream::incoming::{H264IncomingStreamControls, IncomingStreamControls};
use crate::h264_stream::outgoing::{H264StreamControls, StreamControls};
use crate::h264_stream::VIDEO_STREAM_PORT;
use crate::{IncomingVideoStreamControls, OutgoingVideoStreamControls};

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutgoingVideoStreamState {
    On,
    #[default]
    Off,
    Paused,
}
#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum IncomingVideoStreamState {
    On,
    #[default]
    Off,
}
pub struct UILogicPlugin;

impl Plugin for UILogicPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<OutgoingVideoStreamState>();
        app.add_systems(
            Update,
            on_host_button_click.run_if(in_state(OutgoingVideoStreamState::Off)),
        );
    }
}
#[derive(Component, Deref, DerefMut)]
pub struct HostButton(pub IpAddr);

/**************************************/
/************* SYSTEMS ****************/
/**************************************/

fn on_host_button_click(
    query: Query<(&Interaction, &HostButton), Changed<Interaction>>,
    mut v_stream: ResMut<OutgoingVideoStreamControls<H264StreamControls>>,
    mut next_state: ResMut<NextState<OutgoingVideoStreamState>>,
    mut incoming: ResMut<IncomingVideoStreamControls<H264IncomingStreamControls>>,
) {
    for (interaction, addr) in &query {
        if interaction == &Interaction::Pressed {
            // Start streaming video to this host
            let sock_addr = match addr.0 {
                IpAddr::V4(ipv4_addr) => {
                    SocketAddr::V4(SocketAddrV4::new(ipv4_addr, VIDEO_STREAM_PORT))
                }
                IpAddr::V6(ipv6_addr) => {
                    SocketAddr::V6(SocketAddrV6::new(ipv6_addr, VIDEO_STREAM_PORT, 0, 0))
                }
            };
            next_state.set(OutgoingVideoStreamState::On);
            v_stream.0.connect(sock_addr);
            incoming.0.accept(v_stream.0.address).unwrap();
        }
    }
}

fn update_incoming_stream_state() {}
