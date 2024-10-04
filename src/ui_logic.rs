//! Module for UI states and logic.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use bevy::ecs::world::CommandQueue;
use bevy::prelude::*;
use bevy::tasks::futures_lite::future;
use bevy::tasks::{block_on, AsyncComputeTaskPool, Task};
use mdns_sd::ServiceInfo;

use crate::h264_stream::incoming::{H264IncomingStreamControls, IncomingStreamControls};
use crate::h264_stream::outgoing::{H264StreamControls, StreamControls};
use crate::h264_stream::VIDEO_STREAM_PORT;
use crate::ui::{UiContainers, UiSpawner};
use crate::{mdns, STREAM_IMAGE_HANDLE};

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
        app.init_resource::<AvailableHosts>();
        app.add_event::<FindHostsEvent>();
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

        app.add_systems(
            Update,
            update_available_hosts_system.run_if(on_event::<FindHostsEvent>()),
        );
        app.add_systems(Update, handle_tasks);
        app.add_systems(
            Update,
            update_host_list.run_if(resource_changed::<AvailableHosts>),
        );
    }
}

#[derive(Resource, Debug, Default, Deref, DerefMut)]
pub struct AvailableHosts(Vec<ServiceInfo>);

#[derive(Component, Deref, DerefMut)]
pub struct HostButton(pub IpAddr);

#[derive(Component, Deref, DerefMut)]
pub struct ButtonWithRole(pub ButtonRole);
pub enum ButtonRole {
    Disconnect,
    FindHosts,
}
#[derive(Event)]
/// Spawns a task to find the hosts in a non-blocking way. At the end updates the hosts list.
pub struct FindHostsEvent;

#[derive(Component)]
struct UpdateHosts(Task<CommandQueue>);

/**************************************/
/************* SYSTEMS ****************/
/**************************************/

fn update_available_hosts_system(mut commands: Commands) {
    let task_pool = AsyncComputeTaskPool::get();
    let entity = commands.spawn_empty().id();
    let task = task_pool.spawn(async move {
        let hosts = mdns::find_all_hosts();
        info!("{:?}", hosts);
        let mut command_queue = CommandQueue::default();
        command_queue.push(move |world: &mut World| {
            if let Some(mut available_hosts) = world.get_resource_mut::<AvailableHosts>() {
                available_hosts.0 = hosts;
            }
        });
        command_queue
    });
    commands.entity(entity).insert(UpdateHosts(task));
}

fn handle_tasks(mut commands: Commands, mut transform_tasks: Query<(Entity, &mut UpdateHosts)>) {
    for (entity, mut task) in &mut transform_tasks {
        if let Some(mut commands_queue) = block_on(future::poll_once(&mut task.0)) {
            // append the returned command queue to have it execute later
            commands.append(&mut commands_queue);
            commands.entity(entity).despawn_recursive();
        }
    }
}

fn update_host_list(
    mut commands: Commands,
    ui_containers: Res<UiContainers>,
    available_hosts: Res<AvailableHosts>,
    mut spawner: UiSpawner,
) {
    if let Some(mut list) = commands.get_entity(ui_containers.host_bar) {
        list.despawn_descendants();
        for host in &available_hosts.0 {
            let mut btn = spawner.spawn_pretty_button_with_text(host.get_hostname(), 32.);
            if let Some(ip_addr) = host.get_addresses_v4().iter().next() {
                btn.insert(HostButton(IpAddr::V4(**ip_addr)));
            }
            list.add_child(btn.id());
        }
        let mut btn = spawner.spawn_pretty_button_with_text("127.0.0.1", 32.);
        btn.insert(HostButton(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        list.add_child(btn.id());
    }
}

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
            v_stream.0.connect(sock_addr); // Outgoing - sending to 7000
            let mut in_addr = sock_addr;
            in_addr.set_port(6969);
            incoming.0.accept(in_addr).unwrap(); // incoming - connecting to 6969
        }
    }
}

fn check_role_buttons_click(
    query: Query<(&Interaction, &ButtonWithRole), Changed<Interaction>>,
    mut stream_in_state: ResMut<NextState<IncomingVideoStreamState>>,
    mut stream_out_state: ResMut<NextState<OutgoingVideoStreamState>>,
    mut writer: EventWriter<FindHostsEvent>,
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
            ButtonRole::FindHosts => {
                writer.send(FindHostsEvent);
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
