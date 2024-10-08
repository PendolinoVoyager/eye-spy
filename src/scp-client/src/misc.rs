use get_if_addrs::get_if_addrs;
use std::net::IpAddr;

pub fn get_local_ip() -> Option<IpAddr> {
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

pub fn any_as_u8_slice<T: Sized>(p: &T) -> &[u8] {
    unsafe {
        ::core::slice::from_raw_parts((p as *const T) as *const u8, ::core::mem::size_of::<T>())
    }
}
