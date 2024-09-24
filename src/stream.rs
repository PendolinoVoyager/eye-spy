use std::net::UdpSocket;
use std::thread::spawn;
use std::time::Duration;
use v4l::FourCC;

use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::{Device, Format};

/// Module for syscalls / C API's to set up video 4 linux
const WIDTH: u32 = 1920 / 8;
const HEIGHT: u32 = 1080 / 8;
// Using YUV
const FOURCC: FourCC = FourCC { repr: *b"YUYV" };

// and frame ends with 11 one's
const FRAME_END: &[u8] = b"11111111111";
/// The size of packet's raw frame data EXCLUDING meta
const PACKET_DATA_SIZE: u32 = 504;
/// YUYV data frame
pub struct Frame {
    pub finished: bool,
    /// The buffer for the frame
    received: [u8; 1024 * 512],
    /// Missing data in the frame buffer
    missing: Vec<(u32, u32)>,
    last_packet: u32,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            finished: false,
            received: [0; 1024 * 512],
            missing: Vec::with_capacity(10),
            last_packet: 0,
        }
    }
}
impl Frame {
    pub fn new() -> Self {
        Self::default()
    }
    fn clear(&mut self) {
        self.missing.clear();
        self.last_packet = 0;
        self.finished = false;
    }
    /// Add data from the buffer. The more, the better
    pub fn add_data(&mut self, buf: &[u8], n: usize) {
        if buf.starts_with(FRAME_END) && n == 11 {
            self.finished = true;
        } else if let Ok((data, ident)) = Self::decode_frame(&buf[0..n]) {
            // Clear if START frame was missed
            if self.finished || ident <= self.last_packet {
                self.clear();
            }
            dbg!(self.last_packet);
            let missing_packets = ident - self.last_packet;
            if missing_packets > 0 {
                self.handle_missing_packets(missing_packets);
            }
            let offset = missing_packets * PACKET_DATA_SIZE;

            for (idx, byte) in data.iter().enumerate() {
                self.received
                    [((self.last_packet * PACKET_DATA_SIZE + offset) + idx as u32) as usize] =
                    *byte;
            }

            // If packets are missing

            self.last_packet = ident;
        }
    }
    /// Decodes frame. Returns data and identifier
    /// Returned error doesn't matter, we can lose the packet
    fn decode_frame(data: &[u8]) -> Result<(&[u8], u32), ()> {
        if data.len() > 4 {
            let ident_slice = &data[(data.len() - 4)..data.len()];

            let ident = u32::from_le_bytes(ident_slice.try_into().unwrap());
            dbg!(ident);
            return Ok((&data[0..data.len() - 4], ident));
        }
        Err(())
    }
    fn handle_missing_packets(&mut self, missing_packets: u32) {
        for i in 0..=missing_packets {
            let missing_fragment_start = (self.last_packet + i) * PACKET_DATA_SIZE;
            let missing_fragment_end = missing_fragment_start + PACKET_DATA_SIZE;
            self.missing
                .push((missing_fragment_start, missing_fragment_end));
        }
    }
}
pub(crate) fn init_client_streams() {
    let dev = Device::new(0).or(Device::new(1)).unwrap();
    let format = Format::new(WIDTH, HEIGHT, FOURCC);
    dev.set_format(&format).unwrap();
    let mut stream = MmapStream::with_buffers(&dev, Type::VideoCapture, 4)
        .expect("Failed to create buffer stream");

    // Detach both threads
    spawn(move || {
        let udp_receiver = UdpSocket::bind("127.0.0.1:7000").unwrap();
        udp_receiver.connect("127.0.0.1:6969").unwrap();
        let mut recv_buf: [u8; 1024] = [0; 1024];
        let mut frame = Frame::default();
        // Don't do more than 60 fps
        loop {
            while let Ok(bytes_read) = udp_receiver.recv(&mut recv_buf) {
                frame.add_data(&recv_buf, bytes_read);
                if frame.finished {}
            }
        }
    });
    spawn(move || {
        let udp_transmitter = UdpSocket::bind("127.0.0.1:6969").unwrap();
        udp_transmitter.connect("127.0.0.1:7000").unwrap();
        // max safe udp packet is 508 bytes!!!
        // so: send 504 bytes and one int identifier
        loop {
            let (data, _) = stream.next().unwrap();
            if data.is_empty() {
                continue;
            }

            for (num, packet) in data.chunks(PACKET_DATA_SIZE as usize).enumerate() {
                let mut packet_with_ident = Vec::with_capacity(PACKET_DATA_SIZE as usize + 4); // Allocate enough space
                packet_with_ident.extend_from_slice(packet); // Append the packet data
                let num_as_bytes = (num as u32).to_le_bytes(); // Convert num (usize) to 4 bytes (u32)
                packet_with_ident.extend_from_slice(&num_as_bytes); // Append the identifier
                udp_transmitter.send(&packet_with_ident).unwrap();
            }
            udp_transmitter.send(FRAME_END).unwrap();
            std::thread::sleep(Duration::from_micros(16700));
        }
    });
}

#[cfg(test)]
mod tests {
    const FRAME_PATH: &str = "frame.yuyv";
    use crate::stream::PACKET_DATA_SIZE;

    use super::Frame;

    #[test]
    fn test_frame_initialization() {
        let frame = Frame::new();
        assert!(!frame.finished);
        assert_eq!(frame.last_packet, 0);
        assert_eq!(frame.missing.len(), 0);
        assert_eq!(frame.received.len(), 1024 * 512);
        assert!(frame.received.iter().all(|&byte| byte == 0));
    }
    #[test]
    fn test_frame_end_detection() {
        let mut frame = Frame::new();
        let frame_end_buf = b"11111111111"; // Exact match with FRAME_END
        frame.add_data(frame_end_buf, frame_end_buf.len());
        assert!(frame.finished);
    }
    #[test]
    fn test_add_data_to_frame() {
        let mut frame = Frame::new();

        // Simulating a packet with an identifier at the end (e.g., 0x00000001)
        let packet_data: Vec<u8> = vec![0xAB; 500]; // Packet with 500 bytes of data
        let ident: u32 = 0;
        let ident_bytes = ident.to_le_bytes();
        let mut buf = Vec::with_capacity(504);
        buf.extend_from_slice(&packet_data);
        buf.extend_from_slice(&ident_bytes); // Append the identifier

        // Add data to frame
        frame.add_data(&buf, buf.len());

        // Check if data was added at the correct offset
        assert_eq!(frame.last_packet, ident);
        assert!(!frame.finished);

        // Check that the received buffer has been updated
        for (i, &byte) in packet_data.iter().enumerate() {
            dbg!(frame.received[i], byte);
            assert_eq!(frame.received[i], byte);
        }
    }
    #[test]
    fn test_missing_packets_handling() {
        let mut frame = Frame::new();

        // Simulate missing 3 packets
        let missing_packets = 3;
        frame.handle_missing_packets(missing_packets);

        // Check that the missing packets are recorded correctly
        assert_eq!(frame.missing.len(), (missing_packets + 1) as usize); // missing_packets + current one
        for i in 0..=missing_packets {
            let start = (i * PACKET_DATA_SIZE) as usize;
            let end = start + PACKET_DATA_SIZE as usize;
            assert_eq!(frame.missing[i as usize], (start as u32, end as u32));
        }
    }
    #[test]
    fn test_decode_frame() {
        let packet_data: Vec<u8> = vec![0xCD; 500]; // Mock packet data
        let ident: u32 = 2;
        let ident_bytes = ident.to_le_bytes();
        let mut buf = Vec::with_capacity(504);
        buf.extend_from_slice(&packet_data);
        buf.extend_from_slice(&ident_bytes); // Append identifier

        // Decode the frame
        let result = Frame::decode_frame(&buf).unwrap();

        // Check that the data matches
        assert_eq!(result.1, ident);
        assert_eq!(result.0.len(), 500);
        for (i, &byte) in packet_data.iter().enumerate() {
            assert_eq!(result.0[i], byte);
        }
    }
    #[test]
    fn test_frame_clear() {
        let mut frame = Frame::new();

        // Simulate that the frame has some data
        frame.finished = true;
        frame.last_packet = 10;
        frame.missing.push((0, PACKET_DATA_SIZE));

        // Clear the frame
        frame.clear();

        // Check that all fields are reset
        assert!(!frame.finished);
        assert_eq!(frame.last_packet, 0);
        assert_eq!(frame.missing.len(), 0);
    }
}
