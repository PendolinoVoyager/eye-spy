//! Module for UI layout and styling. No logic here.
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use bevy::ecs::system::{EntityCommands, SystemParam};
use bevy::prelude::*;
use bevy_tweening::lens::UiBackgroundColorLens;
use bevy_tweening::{Animator, EaseFunction, Tween};

use crate::ui_logic::HostButton;

// Generic component marking the role of a button
#[derive(Component, Deref, DerefMut, PartialEq, Eq)]
pub struct ButtonMarker(pub u16);
#[allow(unused)]
pub mod color_palette {
    use bevy::color::palettes::tailwind::VIOLET_200;
    use bevy::color::Alpha;
    use bevy::prelude::Color;

    pub const WHITE: Color = Color::srgb(1., 1., 1.);
    pub const DARK: Color = Color::srgba(0.1, 0.1, 0.1, 0.4);
    pub const BLACK: Color = Color::srgba(0., 0., 0., 1.);
}

pub const FONT_PATH: &str = "pixelplay.ttf";

pub struct UIElementsPlugin;

impl Plugin for UIElementsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_fonts);
        app.add_systems(PostStartup, init_ui);
        app.add_systems(Update, pretty_button_behavior);
    }
}

/// Instead of marker components store the constant UI containers in this resource
#[derive(Resource)]
pub struct UiContainers {
    /// Root ui eleement
    root: Entity,
    /// Left side bar with found hosts
    host_bar: Entity,
    /// Contains incoming stream window, eg. use when setting image to placeholder
    stream_window: Entity,
}
/// Marker component for styling behavior
#[derive(Component)]
pub struct PrettyNode;

/// Resource containing systems for spawning and returning Entity ID of a UI Node
#[derive(Resource)]
pub struct UiElementSpawnerResources {
    pub font: Handle<Font>,
}

impl FromWorld for UiElementSpawnerResources {
    fn from_world(world: &mut World) -> Self {
        let font = world.load_asset(FONT_PATH);
        Self { font }
    }
}

/// System to load fonts at startup and insert into `UiElementSpawnerResources`
fn load_fonts(mut commands: Commands, asset_server: Res<AssetServer>) {
    let font_handle = asset_server.load(FONT_PATH);
    commands.insert_resource(UiElementSpawnerResources { font: font_handle });
}

/// A system param to help with spawning UI elements with consistent styles
#[derive(SystemParam)]
pub struct UiSpawner<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub ui_elements: Res<'w, UiElementSpawnerResources>,
}
/// Spawns a button with consistent styling and returns its Entity ID
impl UiSpawner<'_, '_> {
    pub fn spawn_pretty_button(&mut self) -> EntityCommands {
        self.commands.spawn((get_pretty_button(), PrettyNode))
    }
    pub fn spawn_pretty_button_with_text(&mut self, text: &str, font_size: f32) -> EntityCommands {
        let t = self
            .spawn_pretty_text(text, font_size)
            .insert(PrettyNode)
            .id();
        let mut cmds = self.commands.spawn((get_pretty_button(), PrettyNode));
        cmds.add_child(t);
        cmds
    }
    /// Spawns a container with consistent styling and returns its Entity ID
    pub fn spawn_pretty_container(&mut self) -> EntityCommands {
        self.commands.spawn((get_pretty_container(), PrettyNode))
    }

    pub fn spawn_pretty_text(&mut self, text: &str, font_size: f32) -> EntityCommands {
        self.commands.spawn((
            TextBundle::from_section(
                text,
                TextStyle {
                    font_size,
                    font: self.ui_elements.font.clone(),
                    color: color_palette::BLACK,
                },
            ),
            PrettyNode,
        ))
    }
}

/// Function to create a pretty button with predefined styling
fn get_pretty_button() -> ButtonBundle {
    ButtonBundle {
        style: Style {
            padding: UiRect::all(Val::Px(10.)),
            border: UiRect::all(Val::Px(2.)),
            width: Val::Percent(100.),

            display: Display::Flex,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..Default::default()
        },
        z_index: ZIndex::Local(2),
        border_color: BorderColor(color_palette::BLACK),
        background_color: BackgroundColor(color_palette::WHITE),
        ..Default::default()
    }
}

/// Function to create a pretty container with predefined styling
fn get_pretty_container() -> NodeBundle {
    NodeBundle {
        style: Style {
            display: Display::Flex,
            position_type: PositionType::Absolute,
            width: Val::VMax(30.),
            height: Val::VMax(50.),

            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::FlexStart,
            padding: UiRect::all(Val::Percent(2.)),
            row_gap: Val::Percent(5.),

            ..Default::default()
        },
        z_index: ZIndex::Local(1),
        border_color: BorderColor(color_palette::WHITE),
        border_radius: BorderRadius::all(Val::Px(10.)),
        background_color: BackgroundColor(color_palette::DARK),
        ..Default::default()
    }
}

/**************************************/
/************* SYSTEMS ****************/
/**************************************/

#[allow(clippy::type_complexity)]
pub fn pretty_button_behavior(
    query: Query<
        (Entity, &Interaction, &BackgroundColor),
        (Changed<Interaction>, With<PrettyNode>),
    >,
    mut commands: Commands,
    mut window: Query<&mut Window>,
) {
    let mut window = window.single_mut();

    for (entity, interaction, bg) in &query {
        match *interaction {
            Interaction::Pressed => {
                // Scale down the button to indicate it's pressed
                // Optionally, you can change the style as well, e.g., change the border color
                window.cursor.icon = CursorIcon::Default;
                // Play a click sound
            }
            Interaction::Hovered => {
                let tween = Tween::new(
                    EaseFunction::QuadraticIn,
                    Duration::from_millis(200),
                    UiBackgroundColorLens {
                        start: bg.0,
                        end: color_palette::DARK,
                    },
                );
                // Necessary check if entity exists. It may have been deleted as this system doesn't run last
                if let Some(mut e) = commands.get_entity(entity) {
                    e.insert(Animator::new(tween));
                }
                // Tilt the button slightly when hovered

                window.cursor.icon = CursorIcon::Grab;
                // Change the cursor to indicate that the button is interactable
            }
            Interaction::None => {
                let tween = Tween::new(
                    EaseFunction::QuadraticOut,
                    Duration::from_millis(200),
                    UiBackgroundColorLens {
                        start: bg.0,
                        end: color_palette::WHITE,
                    },
                );
                // Necessary check if entity exists. It may have been deleted as this system doesn't run last
                if let Some(mut e) = commands.get_entity(entity) {
                    e.insert(Animator::new(tween));
                }
                window.cursor.icon = CursorIcon::Default;

                // Reset cursor style
            }
        }
    }
}

fn init_ui(mut commands: Commands, mut spawner: UiSpawner) {
    let root = NodeBundle {
        style: Style {
            display: Display::Flex,
            width: Val::Percent(100.),
            height: Val::Percent(100.),
            padding: UiRect::all(Val::Percent(2.)),
            justify_content: JustifyContent::SpaceBetween,
            ..Default::default()
        },
        background_color: BackgroundColor(color_palette::DARK),
        z_index: ZIndex::Global(1),
        ..Default::default()
    };
    let side_bar = NodeBundle {
        style: Style {
            display: Display::Flex,
            width: Val::Percent(30.),
            height: Val::Vh(100.),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.),
            justify_self: JustifySelf::Start,
            ..Default::default()
        },
        ..Default::default()
    };
    let right_side_box = NodeBundle {
        style: Style {
            display: Display::Flex,
            width: Val::Auto,
            height: Val::Vh(100.),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.),
            justify_self: JustifySelf::End,

            ..Default::default()
        },
        ..Default::default()
    };
    let stream_window = commands
        .spawn(NodeBundle {
            style: Style {
                width: Val::Px(crate::h264_stream::WIDTH as f32),
                height: Val::Px(crate::h264_stream::HEIGHT as f32),
                justify_content: JustifyContent::SpaceBetween,
                justify_self: JustifySelf::Center,
                align_self: AlignSelf::Center,
                border: UiRect::all(Val::Px(5.)),

                ..Default::default()
            },

            border_color: BorderColor(color_palette::BLACK),
            ..Default::default()
        })
        .insert(UiImage::new(crate::RGBA_IMAGE_HANDLE).with_flip_x())
        .id();
    let mut root = commands.spawn(root);
    let mut containers = UiContainers {
        root: root.id(),
        stream_window,
        host_bar: Entity::from_raw(0),
    };

    root.with_children(|p| {
        let mut side_bar = p.spawn(side_bar);
        containers.host_bar = side_bar.id();
        let mut btn = spawner.spawn_pretty_button_with_text("127.0.0.1", 32.);
        btn.insert(HostButton(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        side_bar.add_child(btn.id());
        let mut right_bar = p.spawn(right_side_box);
        right_bar.add_child(stream_window);
    });
    commands.insert_resource(containers);
}

// struct TransformRotationLens {
//     start: Quat,
//     end: Quat,
// }

// impl bevy_tweening::Lens<Transform> for TransformRotationLens {
//     fn lerp(
//         &mut self,
//         target: &mut dyn bevy_tweening::Targetable<bevy::prelude::Transform>,
//         ratio: f32,
//     ) {
//         target.rotation = self.start + (self.end - self.start) * ratio;
//     }
// }

// struct TransformScaleLens {
//     start: f32,
//     end: f32,
// }

// impl bevy_tweening::Lens<Transform> for TransformScaleLens {
//     fn lerp(
//         &mut self,
//         target: &mut dyn bevy_tweening::Targetable<bevy::prelude::Transform>,
//         ratio: f32,
//     ) {
//         let start = Vec3::new(self.start, self.start, 0.);
//         let end = Vec3::new(self.end, self.end, 0.);
//         target.scale = start + (end - start) * ratio;
//     }
// }
