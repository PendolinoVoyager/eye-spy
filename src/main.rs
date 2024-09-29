use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use bevy::color::palettes::css::WHITE;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::winit::WinitSettings;

mod h264_stream;
mod ui;
mod ui_logic;

use bevy_tweening::TweeningPlugin;
use h264_stream::incoming::{
    init_incoming_h264_stream, H264IncomingStreamControls, IncomingStreamControls,
};
use h264_stream::outgoing::{init_h264_video_stream, H264StreamControls, StreamControls};
use h264_stream::{HEIGHT, RGB_FRAME_BUFFER, VIDEO_STREAM_PORT, WIDTH};
use ui::UIElementsPlugin;
use ui_logic::OutgoingVideoStreamState;

const RGBA_IMAGE_HANDLE: Handle<Image> = Handle::weak_from_u128(0b00100011010001000101010101101110000011001011010011001111110010000000110000100010001101111111001000011010010010010011001111111101);

/// Newtype for H264 stream controls as Bevy resource
#[derive(Resource)]
pub struct OutgoingVideoStreamControls<T: StreamControls>(T);

#[derive(Resource)]
pub struct IncomingVideoStreamControls<T: IncomingStreamControls>(T);

fn debug_input_stream_controls(
    mut controls: ResMut<OutgoingVideoStreamControls<H264StreamControls>>,
    input: Res<ButtonInput<KeyCode>>,
    mut images: ResMut<Assets<Image>>,
    mut next_state: ResMut<NextState<OutgoingVideoStreamState>>,
    mut incoming: ResMut<IncomingVideoStreamControls<H264IncomingStreamControls>>,
) {
    if input.just_pressed(KeyCode::KeyC) {
        controls
            .0
            .connect(std::net::SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                VIDEO_STREAM_PORT,
            )));
        next_state.set(OutgoingVideoStreamState::On);
    }
    if input.just_pressed(KeyCode::KeyD) {
        controls.0.disconnect();
        if let Some(image) = images.get_mut(&RGBA_IMAGE_HANDLE) {
            image.data.fill(255u8);
        }
        incoming.0.refuse();
        next_state.set(OutgoingVideoStreamState::Off);
    }
    if input.just_pressed(KeyCode::KeyP) {
        controls.0.pause();
    }
    if input.just_pressed(KeyCode::KeyU) {
        controls.0.unpause();
    }
}

fn spawn_preview(mut commands: Commands, mut clear_color: ResMut<ClearColor>) {
    commands.spawn((Camera2dBundle::default(), IsDefaultUiCamera));
    clear_color.0 = WHITE.into();
}
fn get_image_from_rgb(mut images: ResMut<Assets<Image>>) {
    let buf = RGB_FRAME_BUFFER.lock().unwrap();
    let buf = buf.as_slice();
    if buf.is_empty() {
        return;
    }
    let format = TextureFormat::Rgba8UnormSrgb;

    let image = Image::new_fill(
        Extent3d {
            width: WIDTH as u32,
            height: HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        buf,
        format,
        RenderAssetUsages::all(),
    );
    images.insert(RGBA_IMAGE_HANDLE.id(), image);
}

fn main() {
    let addr_out = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 6969);
    let outgoing_controls = init_h264_video_stream(addr_out).unwrap();
    let incoming_controls = init_incoming_h264_stream().unwrap();
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(TweeningPlugin)
        .add_plugins(ui_logic::UILogicPlugin)
        .add_plugins(UIElementsPlugin)
        .insert_resource(OutgoingVideoStreamControls(outgoing_controls))
        .insert_resource(IncomingVideoStreamControls(incoming_controls))
        .insert_resource(Time::<Fixed>::from_seconds(0.050))
        .insert_resource(WinitSettings::game())
        .add_systems(Startup, spawn_preview)
        .add_systems(Update, debug_input_stream_controls)
        .add_systems(
            FixedUpdate,
            get_image_from_rgb.run_if(in_state(OutgoingVideoStreamState::On)),
        )
        .run();

    // Create a texture to store RGB data
}
