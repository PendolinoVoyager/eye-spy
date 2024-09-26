//! This module is responsible for configuring streams from and to the user.
//! It provides basic frame encoding/decoding functionalities

use lazy_static::lazy_static;
use openh264::encoder::{EncodedBitStream, Encoder};
use openh264::formats::YUVSlices;
use std::io::BufWriter;
use std::net::UdpSocket;
use std::sync::Mutex;
use std::thread::spawn;
use v4l::FourCC;

use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::{Device, Format};

const WIDTH: usize = 640;
const HEIGHT: usize = 480;
// Using YUV
const FOURCC: FourCC = FourCC { repr: *b"YUYV" };
/// Packet identifier. Starts with 1
type PacketIdentifier = u32;
// and frame ends with 11 one's
const FRAME_END: &[u8] = b"11111111111";
/// The size of packet's raw frame data EXCLUDING meta
const PACKET_DATA_SIZE: u32 = 504;

// Static buffers so the borrow checker doesn't complain
lazy_static! {
    pub static ref YUV_FRAME_BUFFER_WRITER: Mutex<Vec<u8>> =
        Mutex::new(Vec::with_capacity(WIDTH * HEIGHT * 2));
}
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

    pub fn get_last_frame(&self) -> Option<&[u8; 1024 * 512]> {
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

/// Trait for consistent interfaces accross streams
/// It should be utilized on a wrapper struct for the original stream
pub trait CustomStream<'a, T> {
    fn next(&mut self, buffer: &mut [u8]) -> Option<usize>;
    fn next_vec(&mut self) -> Option<Vec<u8>>;
}

struct H264Stream<'a> {
    stream: MmapStream<'a>,
    encoder: Encoder,
}
impl<'a> H264Stream<'a> {
    pub fn new(device: &Device) -> Self {
        let stream = MmapStream::with_buffers(device, Type::VideoCapture, 4)
            .expect("Failed to create buffer stream");

        let encoder = openh264::encoder::Encoder::new().expect("Cannot create a h264 encoder.");

        Self { stream, encoder }
    }
    #[inline]
    fn prepare_yuv_slices(
        raw_buf: &[u8],
        width: usize,
        height: usize,
    ) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
        let mut y = Vec::with_capacity(width * height);
        let mut u = Vec::with_capacity(width * height / 2);
        let mut v = Vec::with_capacity(width * height / 2);

        // Process the raw YUYV data
        for chunk in raw_buf.chunks(4) {
            // YUYV format: Y1 U Y2 V
            let y0 = chunk[0];
            let u0 = chunk[1];
            let y1 = chunk[2];
            let v0 = chunk[3];
            y.push(y0);
            y.push(y1);
            u.push(u0);
            v.push(v0);
        }
        (y, u, v)
    }

    fn get_encoded_stream(&mut self) -> Result<EncodedBitStream, String> {
        const STRIDES: (usize, usize, usize) = (WIDTH, WIDTH, WIDTH);

        let raw_buf = {
            let (raw_buf, _) = self.stream.next().map_err(|e| e.to_string())?;
            raw_buf
        };

        let slices = Self::prepare_yuv_slices(raw_buf, WIDTH, HEIGHT);

        let slices = YUVSlices::new((&slices.0, &slices.1, &slices.2), (WIDTH, HEIGHT), STRIDES);

        let encoded = self.encoder.encode(&slices).map_err(|e| e.to_string())?;
        Ok(encoded)
    }
}
// H264YUVStream should be thread safe, as it gets data from the ether (/dev/video)
unsafe impl<'a> Send for H264Stream<'a> {}
unsafe impl<'a> Sync for H264Stream<'a> {}

impl CustomStream<'_, MmapStream<'_>> for H264Stream<'_> {
    fn next(&mut self, buffer: &mut [u8]) -> Option<usize> {
        if let Ok(bitstream) = self.get_encoded_stream() {
            // let mut buffer = YUV_FRAME_BUFFER.get_mut().unwrap();
            let mut buf_writer = BufWriter::new(buffer);
            return match bitstream.write(&mut buf_writer) {
                Ok(_) => Some(buf_writer.buffer().len()),

                Err(e) => {
                    dbg!(e);
                    None
                }
            };
        } else {
            None
        }
    }
    fn next_vec(&mut self) -> Option<Vec<u8>> {
        if let Ok(bitstream) = self.get_encoded_stream() {
            let mut vec = Vec::new();
            if bitstream.write(&mut vec).is_err() {
                return None;
            }
            Some(vec)
        } else {
            None
        }
    }
}

pub(crate) fn init_client_streams() {
    let dev = Device::new(0).or(Device::new(1)).unwrap();
    let format = Format::new(WIDTH as u32, HEIGHT as u32, FOURCC);

    dev.set_format(&format).unwrap();

    let mut stream = H264Stream::new(&dev);

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
        let mut buf = Vec::with_capacity(WIDTH * HEIGHT * 2);
        loop {
            let len = stream.next(&mut buf);

            if len.is_none() {
                continue;
            }

            for (num, packet) in buf[0..len.unwrap()]
                .chunks(PACKET_DATA_SIZE as usize)
                .enumerate()
            {
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
    use std::fs::File;
    use std::io::Read;

    use openh264::decoder::Decoder;
    use v4l::video::Capture;
    use v4l::Device;

    use crate::stream::{FOURCC, HEIGHT, WIDTH};

    use super::{CustomStream, FrameBuilder, H264Stream};
    const TEST_H264_FILE: &str = "test.h264";

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
    #[test]
    fn test_frame_encoding() {
        let device = Device::new(0).unwrap();
        let format = v4l::Format::new(WIDTH as u32, HEIGHT as u32, FOURCC);
        device.set_format(&format).unwrap();

        let mut stream = H264Stream::new(&device);
        let buf = stream.next_vec().unwrap();

        assert!(!buf.is_empty(), "Buffer is empty after encoding");

        assert!(
            buf.starts_with(&[0x00, 0x00, 0x00, 0x01]) || buf.starts_with(&[0x00, 0x00, 0x01]),
            "Encoded frame does not start with a valid H264 NAL unit start code"
        );
    }
    #[test]
    fn test_frame_decoding() {
        // Create and open the file
        let mut file = File::open(TEST_H264_FILE).expect("Cannot open the test frame file");

        // Create buffer and buffer reader
        let mut buf: Vec<u8> = Vec::with_capacity(1024 * 100);
        let mut decoder = Decoder::new().unwrap();

        let size = file.read_to_end(&mut buf).unwrap();
        assert!(size > 0);

        let mut frame_ref: Vec<u8> = Vec::new();
        let mut accumulated_data: Vec<u8> = Vec::new(); // Buffer to accumulate NAL units

        // Flags to track if SPS/PPS have been processed
        let mut sps_found = false;
        let mut pps_found = false;

        // Iterate over NAL units
        for packet in openh264::nal_units(&buf[0..size]) {
            let nal_type = packet[0] & 0x1F;

            // SPS NAL unit (NAL type 7)
            if nal_type == 7 {
                sps_found = true;
            }

            // PPS NAL unit (NAL type 8)
            if nal_type == 8 {
                pps_found = true;
            }

            // Accumulate NAL units
            accumulated_data.extend_from_slice(packet);

            // Only start decoding after both SPS and PPS have been processed
            if sps_found && pps_found {
                match decoder.decode(&accumulated_data) {
                    Ok(Some(frame)) => {
                        if frame_ref.is_empty() {
                            frame.write_rgb8(&mut frame_ref);
                        }
                        accumulated_data.clear(); // Clear accumulated data after successful decode
                    }
                    Ok(None) => {
                        // Decoder needs more data, keep accumulating NAL units
                    }
                    Err(e) => {
                        panic!("Decoder error: {:?}", e);
                    }
                }
            }
        }
        dbg!(sps_found, pps_found);
        assert!(
            !frame_ref.is_empty(),
            "Couldn't recover even one frame from the stream."
        );
    }
}
