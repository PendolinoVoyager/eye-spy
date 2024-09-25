//! This module is responsible for configuring streams from and to the user.
//! It provides basic frame encoding/decoding functionalities

use sdl2::sys::Time;
use std::net::UdpSocket;
use std::thread::spawn;
use std::time::Duration;
use v4l::{FourCC, FrameInterval};

use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::{Device, Format};

const WIDTH: u32 = 1920 / 8;
const HEIGHT: u32 = 1080 / 8;
// Using YUV
const FOURCC: FourCC = FourCC { repr: *b"YUYV" };
const FRAMERATE: usize = 30;
/// Packet identifier. Starts with 1
type PacketIdentifier = u32;
// and frame ends with 11 one's
const FRAME_END: &[u8] = b"11111111111";
/// The size of packet's raw frame data EXCLUDING meta
const PACKET_DATA_SIZE: u32 = 504;
/// YUYV data frame
pub struct FrameBuilder {
    pub finished: bool,
    /// The buffer for the frame. To allow mutable access while copying, there are two
    /// Need to be on the heap, stack overflow otherwise
    buffers: [Box<[u8; 1024 * 512]>; 2],
    /// Index of current buffer to write to
    selected_buffer: usize,
    /// Missing data in the frame b/// Lastuffer
    last_packet: PacketIdentifier,
}
impl Default for FrameBuilder {
    fn default() -> Self {
        Self {
            finished: false,
            buffers: [Box::new([0; 1024 * 512]), Box::new([0; 1024 * 512])],
            last_packet: 0,
            selected_buffer: 0,
        }
    }
}
impl FrameBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    fn switch_buffer(&mut self) {
        self.last_packet = 0;
        self.selected_buffer = self.next_buffer();
        self.finished = false;
        let time = std::time::Instant::now();

        dbg!(time, "Frame finished.");
    }

    pub fn get_last_frame(&self) -> Option<&Box<[u8; 1024 * 512]>> {
        let other_buffer = (self.selected_buffer + 1) % self.buffers.len();
        Some(&self.buffers[other_buffer])
    }

    /// Add data from the buffer. The more, the better
    pub fn add_data(&mut self, buf: &[u8], n: usize) {
        if buf.starts_with(FRAME_END) && n == 11 {
            self.finished = true;
        } else if let Ok((data, ident)) = Self::decode_frame(&buf[0..n]) {
            // Clear if START frame was missed
            if self.finished || ident <= self.last_packet {
                self.switch_buffer();
            }
            let missing_packets = ident - 1 - self.last_packet;

            let offset = missing_packets * PACKET_DATA_SIZE;

            for (idx, byte) in data.iter().enumerate() {
                self.buffers[self.selected_buffer]
                    [((self.last_packet * PACKET_DATA_SIZE + offset) + idx as u32) as usize] =
                    *byte;
            }

            // If packets are missing

            self.last_packet = ident;
        }
    }
    /// Gets the next buffer that's supposed to be used, doesn't mutate the current state
    fn next_buffer(&self) -> usize {
        (self.selected_buffer + 1) % self.buffers.len()
    }
    /// Decodes frame. Returns data and identifier
    /// Returned error doesn't matter, we can lose the packet
    fn decode_frame(data: &[u8]) -> Result<(&[u8], u32), ()> {
        if data.len() > 4 {
            let ident_slice = &data[(data.len() - 4)..data.len()];

            let ident = u32::from_le_bytes(ident_slice.try_into().unwrap());
            return Ok((&data[0..data.len() - 4], ident));
        }
        Err(())
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
        let mut frame = FrameBuilder::default();
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
            let (data, meta) = stream.next().unwrap();

            if data.is_empty() || meta.sequence % 2 == 0 {
                continue;
            }

            for (num, packet) in data.chunks(PACKET_DATA_SIZE as usize).enumerate() {
                let mut packet_with_ident = Vec::with_capacity(PACKET_DATA_SIZE as usize + 4); // Allocate enough space
                packet_with_ident.extend_from_slice(packet); // Append the packet data
                let num_as_bytes = (num as u32 + 1).to_le_bytes(); // Convert num (usize) to 4 bytes (u32)
                packet_with_ident.extend_from_slice(&num_as_bytes); // Append the identifier
                udp_transmitter.send(&packet_with_ident).unwrap();
            }
            udp_transmitter.send(FRAME_END).unwrap();
        }
    });
}

// Tests are very important when it comes to manipulating the frame
#[cfg(test)]
mod tests {
    const FRAME_PATH: &str = "frame.yuyv";
    use std::fs::File;
    use std::io::Read;

    use crate::stream::PACKET_DATA_SIZE;

    use super::FrameBuilder;

    #[test]
    fn test_frame_initialization() {
        let frame = FrameBuilder::new();
        assert!(!frame.finished);
        assert_eq!(frame.last_packet, 0);
        assert_eq!(frame.buffers[0].len(), 1024 * 512);
        assert!(frame.buffers[0].iter().all(|&byte| byte == 0));
    }
    #[test]
    fn test_frame_end_detection() {
        let mut frame = FrameBuilder::new();
        let frame_end_buf = b"11111111111"; // Exact match with FRAME_END
        frame.add_data(frame_end_buf, frame_end_buf.len());
        assert!(frame.finished);
    }
    #[test]
    fn test_add_data_to_frame() {
        let mut frame = FrameBuilder::new();

        // Simulating a packet with an identifier at the end (e.g., 0x00000001)
        let packet_data: Vec<u8> = vec![0xAB; 500]; // Packet with 500 bytes of data
        let ident: u32 = 1;
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
            assert_eq!(frame.buffers[0][i], byte);
        }
    }
    #[test]
    fn test_decode_frame() {
        const LENGTH: usize = 102;
        let mut packet_data: [u8; LENGTH] = [0; LENGTH];
        let mut file = File::open(FRAME_PATH).unwrap();
        let _ = file.read(&mut packet_data);
        let ident: u32 = 2;
        let ident_bytes = ident.to_le_bytes();
        let mut buf = Vec::with_capacity(LENGTH);
        buf.extend_from_slice(&packet_data);
        buf.extend_from_slice(&ident_bytes); // Append identifier

        // Decode the frame
        let result = FrameBuilder::decode_frame(&buf).unwrap();

        // Check that the data matches
        assert_eq!(result.1, ident);
        assert_eq!(result.0.len(), LENGTH);
        for (i, &byte) in packet_data.iter().enumerate() {
            assert_eq!(result.0[i], byte);
        }
    }
    #[test]
    fn test_frame_clear() {
        let mut frame = FrameBuilder::new();

        // Simulate that the frame has some data
        frame.finished = true;
        frame.last_packet = 10;
        // Clear the frame
        frame.switch_buffer();

        // Check that all fields are reset
        assert!(!frame.finished);
        assert_eq!(frame.last_packet, 0);
    }
}
