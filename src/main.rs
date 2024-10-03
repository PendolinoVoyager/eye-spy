use std::net::{Ipv4Addr, SocketAddr};

use bevy::color::palettes::css::WHITE;
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::winit::WinitSettings;
mod h264_stream;
mod mdns;
mod scp;
mod ui;
mod ui_logic;

use bevy_tweening::TweeningPlugin;
use h264_stream::incoming::init_incoming_h264_stream;
use h264_stream::outgoing::init_h264_video_stream;
use h264_stream::{HEIGHT, RGB_FRAME_BUFFER, WIDTH};
use ui::UIElementsPlugin;
use ui_logic::{
    IncomingVideoStreamControls, IncomingVideoStreamState, OutgoingVideoStreamControls,
};

pub const STREAM_IMAGE_HANDLE: Handle<Image> = Handle::weak_from_u128(0b00100011010001000101010101101110000011001011010011001111110010000000110000100010001101111111001000011010010010010011001111111101);

fn spawn_preview(mut commands: Commands, mut clear_color: ResMut<ClearColor>) {
    commands.spawn((Camera2dBundle::default(), IsDefaultUiCamera));
    clear_color.0 = WHITE.into();
}
fn update_incoming_stream_image(mut images: ResMut<Assets<Image>>) {
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
    images.insert(STREAM_IMAGE_HANDLE.id(), image);
}

fn main() {
    mdns::start_service();

    let addr_out = SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
    let outgoing_controls = init_h264_video_stream(addr_out).unwrap();
    let incoming_controls = init_incoming_h264_stream().unwrap();

    App::new()
        .insert_resource(OutgoingVideoStreamControls(outgoing_controls))
        .insert_resource(IncomingVideoStreamControls(incoming_controls))
        .add_plugins(DefaultPlugins)
        .add_plugins(TweeningPlugin)
        .add_plugins(ui_logic::UILogicPlugin)
        .add_plugins(UIElementsPlugin)
        .insert_resource(Time::<Fixed>::from_seconds(0.050))
        .insert_resource(WinitSettings::game())
        .add_systems(Startup, spawn_preview)
        .add_systems(
            FixedUpdate,
            update_incoming_stream_image.run_if(in_state(IncomingVideoStreamState::On)),
        )
        .run();

    // Create a texture to store RGB data
}
