//! Module for UI states and logic.

use std::net::{IpAddr, Ipv4Addr};

use bevy::ecs::world::CommandQueue;
use bevy::prelude::*;
use bevy::tasks::futures_lite::future;
use bevy::tasks::{block_on, AsyncComputeTaskPool, Task};
use buttons::{DisconnectButton, FindHostsButton};
use mdns_sd::ServiceInfo;

use crate::connection_state_bevy::{IncomingVideoStreamState, OutgoingVideoStreamState};
use crate::mdns;
use crate::ui::{UiContainers, UiSpawner};

pub struct UILogicPlugin;

impl Plugin for UILogicPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AvailableHosts>();
        app.add_event::<FindHostsEvent>();
        app.add_systems(
            Update,
            on_host_button_click.run_if(in_state(OutgoingVideoStreamState::Off)),
        );
        app.add_systems(Update, (check_disconnect_button, check_find_hosts_button));

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

pub mod buttons {
    use bevy::prelude::Component;
    #[derive(Component)]
    pub struct ConnectButton;
    #[derive(Component)]
    pub struct DisconnectButton;
    #[derive(Component)]
    pub struct FindHostsButton;
    #[derive(Component)]
    pub struct AcceptConnectionButton;
    #[derive(Component)]
    pub struct RejectConnectionButton;
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

/// Spawns a task to try and connect. It will change the state to connecting, and at the end will
/// ConnectionEvent or return the state to off
fn on_host_button_click(query: Query<(&Interaction, &HostButton), Changed<Interaction>>) {
    for (interaction, addr) in &query {
        if interaction == &Interaction::Pressed {
            // Start streaming video to this host
        }
    }
}

fn check_disconnect_button(
    query: Query<&Interaction, (Changed<Interaction>, With<DisconnectButton>)>,
    mut stream_in_state: ResMut<NextState<IncomingVideoStreamState>>,
    mut stream_out_state: ResMut<NextState<OutgoingVideoStreamState>>,
) {
    for interaction in &query {
        if interaction != &Interaction::Pressed {
            continue;
        }
        stream_in_state.set(IncomingVideoStreamState::Off);
        stream_out_state.set(OutgoingVideoStreamState::Off);
    }
}

fn check_find_hosts_button(
    query: Query<&Interaction, (Changed<Interaction>, With<FindHostsButton>)>,
    mut writer: EventWriter<FindHostsEvent>,
) {
    for interaction in &query {
        if interaction != &Interaction::Pressed {
            continue;
        }
        writer.send(FindHostsEvent);
    }
}
