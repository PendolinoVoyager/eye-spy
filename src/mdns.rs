//! This module manages recognition and connections with other apps using mDNS and SCP.

use get_if_addrs::get_if_addrs;
use lazy_static::lazy_static;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Duration;

const SERVICE_NAME: &str = "_eye-spy._tcp.local.";

lazy_static! {
    pub static ref MDNS: ServiceDaemon = ServiceDaemon::new().expect("Failed to create daemon");
    pub static ref MDNS_HOSTS: Mutex<Vec<ServiceInfo>> = Mutex::new(Vec::new());
}

fn get_local_ip() -> Option<IpAddr> {
    let interfaces = get_if_addrs().expect("Failed to get network interfaces");

    for iface in interfaces {
        if !iface.is_loopback() {
            if let IpAddr::V4(ipv4) = iface.ip() {
                return Some(IpAddr::V4(ipv4));
            }
        }
    }

    None
}

/// Starts the mDNS service at this machine.
/// It should be run once at the start somewhere in main()
pub(crate) fn start_service() {
    // Create a service info.
    let instance_name = "my_instance";
    let ip = get_local_ip().expect("Cannot find a network interface that isn't loopback.");
    let host_name = format!("{}.local.", ip.to_string());
    let port = 0;
    let properties = [("in_call", false)];

    let my_service = ServiceInfo::new(
        SERVICE_NAME,
        instance_name,
        &host_name,
        ip,
        port,
        &properties[..],
    )
    .unwrap();
    MDNS.register(my_service)
        .expect("Failed to register our service");
}
/// Finds all hosts of the mDNS service in the network and stores it at MDNS_HOSTS.
/// # Blocking
/// This function blocks the execution until the hosts are found. It has an internal timeout in case something happens.
pub(crate) fn find_all_hosts() -> Vec<ServiceInfo> {
    // Browse for services with the service type you want to find.
    let receiver = MDNS
        .browse(SERVICE_NAME)
        .expect("Failed to browse mDNS services");

    println!("Browsing for mDNS services...");
    let mut new_hosts = Vec::new();

    while let Ok(service_event) = receiver.recv_timeout(Duration::from_secs(1)) {
        match service_event {
            mdns_sd::ServiceEvent::ServiceResolved(service_info) => new_hosts.push(service_info),
            mdns_sd::ServiceEvent::SearchStopped(_) => break,
            _ => (),
        }
    }
    let mut mdns_hosts = MDNS_HOSTS.lock().expect("MDNS_HOSTS poisoned. Exiting.");
    mdns_hosts.clear();
    mdns_hosts.clone_from_slice(&new_hosts);
    new_hosts
}

#[cfg(test)]
pub mod mdns_tests {
    use mdns_sd::{DaemonStatus, Receiver};

    use super::*;
    #[test]
    fn test_get_local_ip() {
        let ip = get_local_ip();
        assert!(ip.is_some(), "No valid IP address found");
        assert!(
            !ip.unwrap().is_loopback(),
            "get_local_ip should not return a loopback address"
        );
    }
    #[test]
    fn test_start_service() {
        start_service();
        assert!(MDNS.status().is_ok_and(
            |v| v.recv_timeout(Duration::from_secs(1)).unwrap() == DaemonStatus::Running
        ));
    }
    #[test]
    fn test_find_hosts() {
        find_all_hosts();
    }
}
