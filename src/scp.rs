//! SCP - Simple Chat Protocol
//! A protocol using mDNS and simple messeges to negotiate stream sessions

use std::fmt::Display;

const SCP_HEADER: &[u8] = b"12345654321\n";
const SCP_END: &[u8] = b"1234564321\n";

#[repr(u8)]
pub enum SCPCommand {
    Start,

    ReqGenerateKey,
    AckGenerateKey,
    KeyShare,

    SimpleMessage,

    VideoStreamConnect,
    AudioStreamConnect,

    VideoStreamStop,
    AudioStreamStop,

    End,
}
pub struct SCPMessage {
    pub body: Vec<u8>,
    pub command: SCPCommand,
}
#[derive(Debug)]
pub enum SCPParseError {
    BadHeader,
    MissingBody,
    MissingEnd,
}
impl Display for SCPParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SCPParseError::BadHeader => f.write_str(&format!(
                "Bad header: SCP message should start with {}",
                String::from_utf8_lossy(SCP_HEADER)
            )),
            SCPParseError::MissingBody => {
                f.write_str("Missing body: Some SCP messages expect body, but found empty")
            }
            SCPParseError::MissingEnd => f.write_str(&format!(
                "No ending: SCP message should end with {}",
                String::from_utf8_lossy(SCP_END)
            )),
        }
    }
}

impl std::error::Error for SCPParseError {}

impl SCPMessage {
    fn deserialize(raw: &[u8]) -> Result<SCPMessage, SCPParseError> {
        if !raw.starts_with(SCP_HEADER) {
            return Err(SCPParseError::BadHeader);
        }

        todo!()
    }
}
