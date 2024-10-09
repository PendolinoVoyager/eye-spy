//! This module is responsible for configuring, encoding, decoding  H264 streams from and to the user.
//! It provides basic controls via <!TODO!>
//! To get a received frame. It works outside any renderer.

use lazy_static::lazy_static;
use openh264::encoder::{EncodedBitStream, Encoder};
use openh264::formats::YUVSlices;

use std::io::BufWriter;
use std::sync::Mutex;

use v4l::FourCC;

use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::Device;
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
/// Port from which YOU receive incoming video stream and connect to to send outgoing
pub const VIDEO_STREAM_PORT: u16 = 7000;

mod ssignal {

    /// Stream Signal None - no signal to stream thread
    pub const SSIGNAL_NONE: u8 = 0;
    /// Stream Signal Disconnect - signal stream to stop until another connection
    pub const SSIGNAL_DISCONNECT: u8 = 1 << 1;
    /// Stream Signal Pause - signal stream thread to pause
    pub const SSIGNAL_PAUSE: u8 = 1 << 2;
    /// Stream Signal Resume - signal stream thread to resume/start  
    pub const SSIGNAL_RESUME: u8 = 1 << 3;
    /// Stream Signal Resume - signal stream thread to resume/start
    /// Loads the SocketAddr from the mutex inside StreamControls.data  
    pub const SSIGNAL_CONNECT: u8 = 1 << 4;
    /// Stream Signal Terminate - signal stream thread to exit loop and terminate  
    pub const SSIGNAL_TERMINATE: u8 = 1 << 5;
}

// Static buffers so the borrow checker doesn't complain
lazy_static! {
    // Only one frame, keep it light-weight and real-time
    pub static ref RGB_FRAME_BUFFER: Mutex<[u8; WIDTH * HEIGHT * 4]> =
        Mutex::new([0; WIDTH * HEIGHT * 4]);
}

/// Trait for consistent interfaces accross streams
/// It should be utilized on a wrapper struct for the original stream
pub trait CustomStream<'a, T> {
    fn next(&mut self, buffer: &mut [u8]) -> Option<usize>;
    fn next_vec(&mut self) -> Option<Vec<u8>>;
}

pub struct H264Stream<'a> {
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
    /// Allocates the buffers for the y u v slices and returns the data.\
    /// # Performance
    /// Compilator actually makes it faster than using statically allocated buffers, somehow...
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

/// Signals passed to the stream thread. The thread will read them the next time the stream loop will run, before any action
/// It will cause delay, but it's easier this way,
/// After reading the signal, it will be set back to SignalNone,
pub(crate) mod outgoing {

    use std::net::{SocketAddr, UdpSocket};
    use std::sync::atomic::{AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use super::ssignal::*;
    use super::{CustomStream, H264Stream};
    use openh264::nal_units;
    use v4l::video::Capture;
    use v4l::{Device, Format};

    /// Context of the thread running the outgoing stream.
    struct OutgoingH264StreamContext<'a> {
        stream: Option<H264Stream<'a>>,
        device: Option<Device>,
        socket: UdpSocket,
        signal: Arc<AtomicU8>,
        signal_data: Arc<Mutex<SocketAddr>>,
        streaming: bool,
        addr_bound: bool,
    }
    impl OutgoingH264StreamContext<'_> {
        fn new(signal: Arc<AtomicU8>, signal_data: Arc<Mutex<SocketAddr>>) -> Self {
            let socket = UdpSocket::bind("127.0.0.1:6969").unwrap();
            socket.set_nonblocking(true).unwrap();

            Self {
                stream: None,
                device: None,
                socket,
                signal,
                signal_data,
                addr_bound: false,
                streaming: false,
            }
        }
        fn process_signals(&mut self) {
            let signal_value = self.signal.load(std::sync::atomic::Ordering::SeqCst);
            let mut op_performed = false;

            match signal_value {
                SSIGNAL_PAUSE => {
                    self.streaming = false;
                    op_performed = true;
                }
                SSIGNAL_DISCONNECT | SSIGNAL_TERMINATE => {
                    self.drop_stream_and_device();
                    self.addr_bound = false;
                    self.streaming = false;
                    op_performed = signal_value == SSIGNAL_DISCONNECT;
                }
                SSIGNAL_CONNECT => {
                    if let Ok(addr) = self.signal_data.lock() {
                        if let Err(err) = self.socket.connect(addr.to_string()) {
                            eprintln!(
                                "Cannot connect to socket waiting for H264 stream: {:?}",
                                err
                            );
                            return;
                        }

                        self.streaming = true;
                        self.addr_bound = true;
                        if self.stream.is_none() || self.device.is_none() {
                            let (new_stream, new_dev) = init_inner_stream();
                            self.stream = Some(new_stream);
                            self.device = Some(new_dev);
                        }
                        // Force an intra-frame
                        if let Some(ref mut stream_ref) = self.stream {
                            stream_ref.encoder.force_intra_frame();
                        }

                        op_performed = true;
                    }
                }
                SSIGNAL_RESUME => {
                    self.streaming = true;
                    op_performed = true;
                }
                _ => {}
            }

            // Reset the signal if an operation was performed
            if op_performed {
                self.signal
                    .store(SSIGNAL_NONE, std::sync::atomic::Ordering::SeqCst);
            }
        }

        fn drop_stream_and_device(&mut self) {
            self.stream.take();
            self.device.take();
        }
    }

    pub trait StreamControls {
        /// Connect to an address to send data to from given port
        /// After connecting the stream will be in paused state.
        fn connect(&mut self, addr: SocketAddr);
        /// Disconnect and terminate the stream thread.
        fn disconnect(&mut self);
        /// Pause the stream if connected, with ability to unpause later
        fn pause(&mut self);
        /// Unpause the stream after pausing, or start if just connected
        fn unpause(&mut self);
    }

    pub struct H264StreamControls {
        t_handle: JoinHandle<()>,
        /// Atomic for frequent reads
        signal: Arc<AtomicU8>,
        /// Mutex for storing SocketAddr once
        signal_data: Arc<Mutex<SocketAddr>>,
        pub address: SocketAddr,
    }
    impl H264StreamControls {
        fn new(
            t: JoinHandle<()>,
            signal: Arc<AtomicU8>,
            signal_data: Arc<Mutex<SocketAddr>>,
            address: SocketAddr,
        ) -> Self {
            Self {
                t_handle: t,
                signal,
                signal_data,
                address,
            }
        }
    }
    impl StreamControls for H264StreamControls {
        fn connect(&mut self, addr: SocketAddr) {
            let mut data_guard = self.signal_data.try_lock().unwrap();

            *data_guard = addr;
            self.signal.store(SSIGNAL_CONNECT, Ordering::SeqCst);
        }

        fn disconnect(&mut self) {
            self.signal.store(SSIGNAL_DISCONNECT, Ordering::SeqCst);
        }

        fn pause(&mut self) {
            self.signal.store(SSIGNAL_PAUSE, Ordering::SeqCst);
        }

        fn unpause(&mut self) {
            self.signal.store(SSIGNAL_RESUME, Ordering::SeqCst);
        }
    }
    impl Drop for H264StreamControls {
        fn drop(&mut self) {
            self.signal.store(SSIGNAL_TERMINATE, Ordering::SeqCst);
        }
    }
    /// Inits a new stream, including opening the video device.

    fn init_inner_stream<'a>() -> (H264Stream<'a>, Device) {
        let dev = Device::new(0).or(Device::new(1)).unwrap();
        let format = Format::new(super::WIDTH as u32, super::HEIGHT as u32, super::FOURCC);
        dev.set_format(&format).unwrap();

        let stream = H264Stream::new(&dev);
        (stream, dev)
    }
    /// Init the video stream. Returns controls to the stream, or Error
    /// The socket will be created at given address
    pub(crate) fn init_h264_video_stream(addr: SocketAddr) -> Result<H264StreamControls, ()> {
        let signal = Arc::new(AtomicU8::new(SSIGNAL_NONE));

        let signal_data = Arc::new(Mutex::new(addr)); // Protect the address with a Mutex

        // Clone Arc to be used in the thread
        let signal_clone = Arc::clone(&signal);
        let signal_data_clone = Arc::clone(&signal_data);

        // Spawn a thread to control the stream
        let t = std::thread::spawn(move || {
            let mut stream_context =
                OutgoingH264StreamContext::new(signal_clone, signal_data_clone);

            loop {
                stream_context.process_signals();

                if !stream_context.streaming || !stream_context.addr_bound {
                    //  signal terminate won't be "taken" after reading, persisting after processing
                    //  process_signals() only shuts down the thing, breaking has to be done inside the loop
                    if stream_context.signal.load(Ordering::Relaxed) == SSIGNAL_TERMINATE {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(30));

                    continue;
                }

                if let Some(ref mut stream_ref) = stream_context.stream {
                    if let Some(buf) = stream_ref.next_vec() {
                        for unit in nal_units(&buf) {
                            for (num, packet) in
                                unit.chunks(super::PACKET_DATA_SIZE as usize).enumerate()
                            {
                                // This vector is nicely optimized by the compiler. No need for a buffer
                                let mut packet_with_ident =
                                    Vec::with_capacity(super::PACKET_DATA_SIZE as usize + 4);
                                packet_with_ident.extend_from_slice(packet); // Append the packet data
                                let num_as_bytes = (num as u32 + 1).to_le_bytes(); // Convert num (usize) to 4 bytes (u32)
                                packet_with_ident.extend_from_slice(&num_as_bytes); // Append the identifier

                                let _ = stream_context.socket.send(&packet_with_ident);
                            }
                            let _ = stream_context.socket.send(super::FRAME_END);
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });

        let controls = H264StreamControls::new(t, signal, signal_data, addr);
        Ok(controls)
    }
}

//////////////////////////////////
//INCOMING STREAM CONTROLS ///////
//////////////////////////////////
pub mod incoming {

    use anyhow::Error;
    use openh264::decoder::Decoder;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
    use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    use super::{ssignal::*, VIDEO_STREAM_PORT};
    use super::{PacketIdentifier, FRAME_END, HEIGHT, RGB_FRAME_BUFFER, WIDTH};

    const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
    const SINGLE_READ_TIMEOUT: Duration = Duration::from_millis(100);

    /// If no new frames arrive within this time, the connection is dropped
    // const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

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

    pub trait IncomingStreamControls {
        /// Accept connections from a host
        fn accept(&mut self, addr: SocketAddr) -> anyhow::Result<()>;
        /// Refuse the connection (i.e. stop)
        fn refuse(&mut self);
        /// Get the latest WHOLE piece of data (frame, audio fragment)
        fn get_data(&self) -> anyhow::Result<&[u8]>;
        /// Check if the host might be down
        fn is_receiving(&self) -> bool;
    }
    /// Controls for incoming stream.

    pub struct H264IncomingStreamControls {
        t_handle: JoinHandle<()>,
        signal: Arc<AtomicU8>,
        signal_data: Arc<Mutex<SocketAddr>>,
        conn_status: Arc<AtomicBool>,
    }

    impl H264IncomingStreamControls {
        /// Create new UDP socket to listen to incoming video stream on const VIDEO_STREAM_PORT
        /// Additionally, it spawns a thread to listen to incoming data
        /// # Errors
        /// Might return an error if the socket cannot be bound
        pub fn new(
            t_handle: JoinHandle<()>,
            signal: Arc<AtomicU8>,
            signal_data: Arc<Mutex<SocketAddr>>,
            conn_status: Arc<AtomicBool>,
        ) -> Self {
            Self {
                conn_status,
                t_handle,
                signal,
                signal_data,
            }
        }
    }
    impl Drop for H264IncomingStreamControls {
        fn drop(&mut self) {
            self.signal
                .store(SSIGNAL_TERMINATE, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl IncomingStreamControls for H264IncomingStreamControls {
        /// Accept a new connection. If a connection exists, it's overridden.
        fn accept(&mut self, addr: SocketAddr) -> anyhow::Result<()> {
            let lock = self.signal_data.lock();
            // stupid error cannot be send between threads safely
            if lock.is_err() {
                return Err(Error::msg(
                    "Cannot acquire the signal lock for incoming h.264 stream.",
                ));
            }
            let mut lock = lock.unwrap();
            *lock = addr;
            self.signal
                .store(SSIGNAL_CONNECT, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        /// Disconnect from the current stream.
        fn refuse(&mut self) {
            self.signal
                .store(SSIGNAL_DISCONNECT, std::sync::atomic::Ordering::SeqCst);
        }

        fn get_data(&self) -> anyhow::Result<&[u8]> {
            let lock = RGB_FRAME_BUFFER
                .lock()
                .map_err(|_| Error::msg("Mutex poisoned"))?;
            todo!();
        }

        fn is_receiving(&self) -> bool {
            self.conn_status.load(Ordering::SeqCst)
        }
    }

    /// Initializes the required parts to get an incoming stream working.
    /// Returns controls to the incoming stream.
    pub(crate) fn init_incoming_h264_stream() -> anyhow::Result<H264IncomingStreamControls> {
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, VIDEO_STREAM_PORT));

        let socket = UdpSocket::bind(addr)?;
        socket.set_read_timeout(Some(SINGLE_READ_TIMEOUT)).unwrap();

        let signal = Arc::new(AtomicU8::new(SSIGNAL_NONE));
        let signal_data = Arc::new(Mutex::new(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            10000,
        )));
        let conn_status = Arc::new(AtomicBool::new(false));

        let signal_clone = Arc::clone(&signal);
        let signal_data_clone = Arc::clone(&signal_data);
        let conn_status_clone = Arc::clone(&conn_status);

        // Spawn the data processing thread
        let t = thread::spawn(move || {
            let mut recv_buf: [u8; 1024] = [0; 1024];
            let mut nal_builder = NalBuilder::new();
            let mut decoder = Decoder::new().unwrap();
            let mut last_packet = Instant::now();

            loop {
                // read signals first
                match signal_clone.load(std::sync::atomic::Ordering::SeqCst) {
                    SSIGNAL_CONNECT => {
                        //get addr from signal_data_clone.
                        let addr = signal_data_clone.lock().unwrap();

                        if socket.connect(*addr).is_ok() {
                            signal_clone.store(SSIGNAL_NONE, Ordering::SeqCst);
                            nal_builder.reset();
                            let _ = socket.take_error();
                            conn_status_clone.store(true, Ordering::SeqCst);
                        }
                    }
                    SSIGNAL_DISCONNECT => {
                        signal_clone.store(SSIGNAL_NONE, Ordering::SeqCst);

                        conn_status_clone.store(false, Ordering::SeqCst);
                    }

                    SSIGNAL_TERMINATE => {
                        break;
                    }
                    _ => (),
                };

                if !conn_status_clone.load(Ordering::Relaxed) {
                    // Sleep briefly if not connected
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                // Data reception - timeout is 100ms

                if let Ok(bytes_read) = socket.recv(&mut recv_buf) {
                    last_packet = Instant::now();
                    nal_builder.add_data(&recv_buf[0..bytes_read]);
                    if let Some(unit) = nal_builder.get_nal_unit() {
                        if let Ok(Some(d)) = decoder.decode(unit) {
                            d.write_rgba8(
                                &mut RGB_FRAME_BUFFER.lock().unwrap()[0..(WIDTH * HEIGHT * 4)],
                            );
                        }
                    }
                } else if last_packet.duration_since(Instant::now()) > CONNECTION_TIMEOUT {
                    conn_status_clone.store(false, Ordering::SeqCst);
                }
            }
        });
        let controls = H264IncomingStreamControls::new(t, signal, signal_data, conn_status);
        Ok(controls)
    }
}

#[cfg(test)]
mod tests {

    use openh264::decoder::Decoder;
    use v4l::video::Capture;
    use v4l::Device;

    use crate::h264_stream::{FOURCC, HEIGHT, WIDTH};

    use super::{CustomStream, H264Stream};

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

        let mut frame_ref: [u8; WIDTH * HEIGHT * 4] = [0; WIDTH * HEIGHT * 4];
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
                        frame.write_rgba8(&mut frame_ref);
                    }
                    accumulated_data.clear(); // Clear accumulated data after successful decode
                }
                Ok(None) => (),
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
