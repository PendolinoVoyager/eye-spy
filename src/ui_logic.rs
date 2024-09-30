//! Module for UI states and logic.

use std::net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6};

use bevy::prelude::*;

use crate::h264_stream::incoming::{H264IncomingStreamControls, IncomingStreamControls};
use crate::h264_stream::outgoing::{H264StreamControls, StreamControls};
use crate::h264_stream::VIDEO_STREAM_PORT;
use crate::STREAM_IMAGE_HANDLE;

/// Newtype for H264 stream controls as Bevy resource
#[derive(Resource)]
pub struct OutgoingVideoStreamControls<T: StreamControls>(pub T);

#[derive(Resource)]
pub struct IncomingVideoStreamControls<T: IncomingStreamControls>(pub T);

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
pub struct UILogicPlugin;

impl Plugin for UILogicPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<OutgoingVideoStreamState>();
        app.init_state::<IncomingVideoStreamState>();

        app.add_systems(
            Update,
            on_host_button_click.run_if(in_state(OutgoingVideoStreamState::Off)),
        );
        app.add_systems(Update, check_role_buttons_click);
        app.add_systems(
            OnEnter(OutgoingVideoStreamState::Off),
            on_disconnect_out_stream,
        );
        app.add_systems(
            OnEnter(IncomingVideoStreamState::Off),
            on_disconnect_in_stream,
        );
    }
}
#[derive(Component, Deref, DerefMut)]
pub struct HostButton(pub IpAddr);

#[derive(Component, Deref, DerefMut)]
pub struct ButtonWithRole(pub ButtonRole);
pub enum ButtonRole {
    Disconnect,
}

/**************************************/
/************* SYSTEMS ****************/
/**************************************/

fn on_host_button_click(
    query: Query<(&Interaction, &HostButton), Changed<Interaction>>,
    mut v_stream: ResMut<OutgoingVideoStreamControls<H264StreamControls>>,
    mut stream_in_state: ResMut<NextState<IncomingVideoStreamState>>,
    mut stream_out_state: ResMut<NextState<OutgoingVideoStreamState>>,
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
            stream_out_state.set(OutgoingVideoStreamState::On);
            stream_in_state.set(IncomingVideoStreamState::On);
            v_stream.0.connect(sock_addr);
            incoming.0.accept(v_stream.0.address).unwrap();
        }
    }
}

fn check_role_buttons_click(
    query: Query<(&Interaction, &ButtonWithRole), Changed<Interaction>>,
    mut stream_in_state: ResMut<NextState<IncomingVideoStreamState>>,
    mut stream_out_state: ResMut<NextState<OutgoingVideoStreamState>>,
) {
    for (interaction, role) in &query {
        if interaction != &Interaction::Pressed {
            continue;
        }
        match role.0 {
            ButtonRole::Disconnect => {
                stream_in_state.set(IncomingVideoStreamState::Off);
                stream_out_state.set(OutgoingVideoStreamState::Off);
            }
        }
    }
}

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
