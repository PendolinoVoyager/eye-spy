use std::net::{Ipv4Addr, SocketAddrV4};

use bevy::color::palettes::css::WHITE;
use bevy::color::palettes::tailwind;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;
pub(crate) mod h264_stream;

use bevy::render::render_asset::RenderAssetUsages;
use bevy::render::render_resource::{Extent3d, TextureFormat};

use bevy::winit::WinitSettings;
use h264_stream::stream_control::{H264StreamControls, StreamControls};
use h264_stream::{init_h264_video_stream, start_debug_listener, HEIGHT, RGB_FRAME_BUFFER, WIDTH};

const RGBA_IMAGE_HANDLE: Handle<Image> = Handle::weak_from_u128(0b00100011010001000101010101101110000011001011010011001111110010000000110000100010001101111111001000011010010010010011001111111101);

/// Newtype for H264 stream controls as Bevy resource
#[derive(Resource)]
pub struct VideoStreamControls(pub H264StreamControls);

fn debug_input_stream_controls(
    mut controls: ResMut<VideoStreamControls>,
    input: Res<ButtonInput<KeyCode>>,
    mut images: ResMut<Assets<Image>>,
) {
    if input.just_pressed(KeyCode::KeyC) {
        controls
            .0
            .connect(std::net::SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::LOCALHOST,
                7000,
            )));
    }
    if input.just_pressed(KeyCode::KeyD) {
        controls.0.disconnect();
        if let Some(image) = images.get_mut(&RGBA_IMAGE_HANDLE) {
            image.data.fill(255u8);
        }
    }
    if input.just_pressed(KeyCode::KeyP) {
        controls.0.pause();
    }
    if input.just_pressed(KeyCode::KeyP) {
        controls.0.unpause();
    }
}

fn spawn_preview(mut commands: Commands, mut clear_color: ResMut<ClearColor>) {
    commands.spawn((Camera2dBundle::default(), IsDefaultUiCamera));
    clear_color.0 = WHITE.into();
    commands
        .spawn(NodeBundle {
            style: Style {
                width: Val::Percent(50.),
                height: Val::Percent(50.),
                justify_content: JustifyContent::SpaceBetween,
                justify_self: JustifySelf::Center,
                align_self: AlignSelf::Center,
                border: UiRect::all(Val::Px(5.)),

                ..Default::default()
            },

            border_color: BorderColor(tailwind::BLUE_500.into()),

            ..Default::default()
        })
        .insert(UiImage::new(RGBA_IMAGE_HANDLE).with_flip_x());
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

#[derive(States, Default, Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutgoingVideoStreamState {
    On,
    #[default]
    Off,
    Paused,
}
fn main() {
    let mut controls = init_h264_video_stream().unwrap();
    // Listening on 7000
    start_debug_listener();

    App::new()
        // .insert_state(OutgoingVideoStreamState::Off)
        .insert_resource(VideoStreamControls(controls))
        .insert_resource(Time::<Fixed>::from_seconds(0.050))
        .add_plugins(DefaultPlugins)
        .insert_resource(WinitSettings::game())
        .add_systems(Startup, spawn_preview)
        .add_systems(Update, debug_input_stream_controls)
        .add_systems(FixedUpdate, get_image_from_rgb)
        .run();

    // Create a texture to store RGB data
}
