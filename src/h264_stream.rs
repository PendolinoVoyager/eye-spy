//! This module is responsible for configuring, encoding, decoding  H264 streams from and to the user.
//! It provides basic controls via <!TODO!>
//! To get a received frame

use lazy_static::lazy_static;
use openh264::decoder::Decoder;
use openh264::encoder::{EncodedBitStream, Encoder};
use openh264::formats::YUVSlices;
use openh264::nal_units;
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
pub const WIDTH: usize = 640;
pub const HEIGHT: usize = 480;
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
    pub static ref RGB_FRAME_BUFFER: Mutex<[u8; WIDTH * HEIGHT * 3]> =
        Mutex::new([0; WIDTH * HEIGHT * 3]);
}

/// NAL unit builder for a H.264 stream over UDP.
/// The NAL units cannot be safely sent over UDP without splitting them into smaller packets.
/// This object uses the underlying decoder only when the whole NAL unit can be re-created.
/// TODO: UDP PACKETS MIGHT COME UNORDERED, SAVE THE LAST N OF THEM AND LOOK IF CAN RECOVER
pub struct NalBuilder {
    pub finished: bool,
    pub failed: bool,
    /// The buffer for the nal unit. For safety purposes, it's set to the max NAL unit size possible
    nal_unit_buffer: Box<[u8; 65535]>,
    /// Identifier of the last packet. If the packet is lost, the NAL unit build is failed
    last_packet: PacketIdentifier,
    end_idx: usize,
    last_idx: usize,
}
impl Default for NalBuilder {
    fn default() -> Self {
        Self {
            finished: false,
            failed: false,
            nal_unit_buffer: Box::new([0; 65535]),
            last_packet: 0,
            end_idx: 0,
            last_idx: 0,
        }
    }
}
impl NalBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get_nal_unit(&self) -> Option<&[u8]> {
        if self.finished && !self.failed {
            Some(&self.nal_unit_buffer[0..self.end_idx])
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.finished = false;
        self.failed = false;
        self.last_packet = 0;
        self.end_idx = 0;
        self.last_idx = 0;
    }
    /// Add data from the buffer. The more, the better
    pub fn add_data(&mut self, buf: &[u8]) {
        if buf.starts_with(FRAME_END) && buf.len() == 11 {
            self.finished = true;
        } else if let Ok((data, ident)) = Self::decode_frame(buf) {
            if self.finished || ident <= self.last_packet {
                self.reset();
            }
            if self.failed {
                return;
            }
            let missing_packets = ident - 1 - self.last_packet;
            if missing_packets > 0 {
                self.failed = true;
                return;
            };
            self.last_packet = ident;
            // Copy the data into the buffer at correct slot
            for byte in data.iter() {
                self.nal_unit_buffer[self.last_idx] = *byte;
                self.last_idx += 1;
            }
            self.end_idx += data.len();
        }
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
        let buffer = self.stream.next().map_err(|e| e.to_string())?.0;

        let slices = Self::prepare_yuv_slices(buffer, WIDTH, HEIGHT);
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
        let udp_transmitter = UdpSocket::bind("127.0.0.1:6969").unwrap();
        udp_transmitter.connect("127.0.0.1:7000").unwrap();
        // Sending NAL packets, which can be lost
        // let mut buf = Vec::with_capacity(WIDTH * HEIGHT * 2);
        loop {
            let buf = stream.next_vec();

            if buf.is_none() {
                continue;
            }

            for unit in nal_units(&buf.unwrap()) {
                for (num, packet) in unit.chunks(PACKET_DATA_SIZE as usize).enumerate() {
                    let mut packet_with_ident = Vec::with_capacity(PACKET_DATA_SIZE as usize + 4); // Allocate enough space
                    packet_with_ident.extend_from_slice(packet); // Append the packet data
                    let num_as_bytes = (num as u32 + 1).to_le_bytes(); // Convert num (usize) to 4 bytes (u32)
                    packet_with_ident.extend_from_slice(&num_as_bytes); // Append the identifier

                    udp_transmitter.send(&packet_with_ident).unwrap();
                }
                udp_transmitter.send(FRAME_END).unwrap();
            }
        }
    });
    spawn(move || {
        let udp_receiver = UdpSocket::bind("127.0.0.1:7000").unwrap();
        udp_receiver.connect("127.0.0.1:6969").unwrap();
        let mut recv_buf: [u8; 1024] = [0; 1024];
        // Don't do more than 60 fps
        let mut nal_builder = NalBuilder::new();

        let mut decoder = Decoder::new().unwrap();
        loop {
            while let Ok(bytes_read) = udp_receiver.recv(&mut recv_buf) {
                nal_builder.add_data(&recv_buf[0..bytes_read]);
                if nal_builder.finished && !nal_builder.failed {
                    let unit = nal_builder.get_nal_unit();
                    if unit.is_none() {
                        continue;
                    }

                    match decoder.decode(unit.unwrap()) {
                        Err(e) => (),
                        Ok(Some(d)) => d.write_rgb8(
                            &mut RGB_FRAME_BUFFER.lock().unwrap()[0..(WIDTH * HEIGHT * 3)],
                        ),
                        Ok(None) => println!("No frame..."),
                    }
                }
            }
        }
    });
}

// Tests are very important when it comes to manipulating the frame
#[cfg(test)]
mod tests {

    use openh264::decoder::Decoder;
    use v4l::video::Capture;
    use v4l::Device;

    use crate::h264_stream::{FOURCC, HEIGHT, WIDTH};

    use super::{CustomStream, H264Stream};
    const TEST_H264_FILE: &str = "test.h264";

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
        // encoded h264 stream
        let bytes = include_bytes!("../test.h264");
        let mut decoder = Decoder::new().unwrap();

        let mut frame_ref: [u8; WIDTH * HEIGHT * 3] = [0; WIDTH * HEIGHT * 3];
        let mut accumulated_data: Vec<u8> = Vec::new(); // Buffer to accumulate NAL units

        // Flags to track if SPS/PPS have been processed
        // Iterate over NAL units
        for unit in openh264::nal_units(bytes) {
            // Accumulate NAL units
            accumulated_data.extend_from_slice(unit);

            // Only start decoding after both SPS and PPS have been processed
            match decoder.decode(&accumulated_data) {
                Ok(Some(frame)) => {
                    if frame_ref.is_empty() {
                        frame.write_rgb8(&mut frame_ref);
                    }
                    accumulated_data.clear(); // Clear accumulated data after successful decode
                }
                Ok(None) => {
                    dbg!("NONE");
                }
                Err(e) => {
                    panic!("Decoder error: {:?}", e);
                }
            }
        }
        assert!(
            !frame_ref.is_empty(),
            "Couldn't recover even one frame from the stream."
        );
    }
}
