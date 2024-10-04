//! Contains the implementation of ScpClient
//! # Examples
//! ```
//! let client = ScpClient::new();
//! Yadada :)
//! ```
#![allow(unused)]
use std::any::TypeId;
use std::net::{IpAddr, TcpListener};
use std::thread::JoinHandle;

/// Events used by the client in the internal event-loop to manage the state of connection
pub enum ScpEvent {
    Handshake,
    EncryptionSet,
    StreamsConfigured,
    Ready,
    End,
}
/// Configuration for an established chat session
/// These are "suggestions" only and the responsibility to use all of them correctly
/// falls on the external implementation.
/// * `ip` - IpAddr of the connection
/// * `port_video` - UDP port to send video stream to
/// * `port_audio` - UDP port to send audio stream to
/// * `video_encoding` - !UNUSED! method of video encoding used
/// * `audio_encoding` - !UNUSED! method of audio encoding used
/// * `encryption_key` - encryption key used to encrypt all and any packets sent
/// * `encryption_method` - !UNUSED! - encryption method used
#[derive(Clone, Debug)]
pub struct SessionConfig {
    pub encryption_key: Option<String>,
    pub encrytpion_method: Option<bool>,
    pub ip: IpAddr,
    pub port_video: Option<u16>,
    pub port_audio: Option<u16>,
    pub video_encoding: VideoEncoding,
    pub audio_encoding: AudioEncoding,
}
/// Available video encoding formats
#[derive(Clone, Copy, Debug)]
pub enum VideoEncoding {
    H264,
}
/// Available audio encoding formats
#[derive(Clone, Copy, Debug)]
pub enum AudioEncoding {
    NoIdea,
}

struct Preferences {
    video_encoding: VideoEncoding,
    audio_encoding: AudioEncoding,
    port_in_video: u16,
    port_in_audio: u16,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            video_encoding: VideoEncoding::H264,
            audio_encoding: AudioEncoding::NoIdea,
            port_in_audio: 7001,
            port_in_video: 7000,
        }
    }
}
pub struct ScpClient {
    last_config: Option<SessionConfig>,
    listener: TcpListener,
    thread: JoinHandle<()>,
    preferences: Preferences,
}

impl ScpClient {
    pub fn new() -> Self {
        todo!()
    }
    fn with_preferences(preferences: Preferences) -> Self {
        todo!()
    }
    pub fn request_chat() -> Option<SessionConfig> {
        todo!()
    }
    fn init_connection() {}
}

/// Convinient builder for ScpClient with preferences
pub struct ScpClientBuilder {
    preferences: Preferences,
}
impl ScpClientBuilder {
    pub fn new() -> Self {
        Self {
            preferences: Preferences::default(),
        }
    }
    pub fn build(self) -> ScpClient {
        ScpClient::with_preferences(self.preferences)
    }
    pub fn video_port(self, port: u16) -> Self {
        Self {
            preferences: Preferences {
                port_in_video: port,
                ..self.preferences
            },
        }
    }
    pub fn audio_port(self, port: u16) -> Self {
        Self {
            preferences: Preferences {
                port_in_audio: port,
                ..self.preferences
            },
        }
    }
    pub fn video_encoding(self, encoding: VideoEncoding) -> Self {
        Self {
            preferences: Preferences {
                video_encoding: encoding,
                ..self.preferences
            },
        }
    }
    pub fn audio_encoding(self, encoding: AudioEncoding) -> Self {
        Self {
            preferences: Preferences {
                audio_encoding: encoding,
                ..self.preferences
            },
        }
    }
}
